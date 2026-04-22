use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

use arboard::{Clipboard, ImageData};
use tokio::task;

use crate::{
    error::{AppError, AppResult},
    sync::protocol::{
        ClipboardContent, ClipboardDispatch, ClipboardPayload, FileBundle, FileBundleFile,
        ImageFrame, LocalFileSource,
    },
};

const MAX_FILE_COUNT: usize = 24;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_FILE_BYTES: u64 = 512 * 1024 * 1024;

pub async fn read_content(device_id: uuid::Uuid) -> AppResult<Option<ClipboardDispatch>> {
    task::spawn_blocking(move || {
        let mut clipboard = Clipboard::new().map_err(|error| AppError::Clipboard(error.to_string()))?;

        if let Ok(html) = clipboard.get().html() {
            let plain_text = clipboard.get_text().ok();
            return Ok(Some(dispatch(
                device_id,
                ClipboardContent::Html { html, plain_text },
                Vec::new(),
            )));
        }

        if let Ok(paths) = clipboard.get().file_list() {
            if !paths.is_empty() {
                if let Some((bundle, local_files)) = build_file_bundle(paths)? {
                    return Ok(Some(dispatch(
                        device_id,
                        ClipboardContent::Files { bundle },
                        local_files,
                    )));
                }
            }
        }

        if let Ok(image) = clipboard.get_image() {
            return Ok(Some(dispatch(
                device_id,
                ClipboardContent::Image {
                    image: ImageFrame {
                        width: image.width,
                        height: image.height,
                        png_bytes: encode_png_rgba(image.width, image.height, image.bytes.as_ref())?,
                    },
                },
                Vec::new(),
            )));
        }

        match clipboard.get_text() {
            Ok(text) => Ok(Some(dispatch(
                device_id,
                ClipboardContent::Text { text },
                Vec::new(),
            ))),
            Err(_) => Ok(None),
        }
    })
    .await
    .map_err(|error| AppError::Clipboard(error.to_string()))?
}

pub async fn write_content(content: ClipboardContent, transfer_id: Option<String>) -> AppResult<()> {
    task::spawn_blocking(move || {
        let mut clipboard = Clipboard::new().map_err(|error| AppError::Clipboard(error.to_string()))?;
        match content {
            ClipboardContent::Text { text } => clipboard
                .set_text(text)
                .map_err(|error| AppError::Clipboard(error.to_string())),
            ClipboardContent::Html { html, plain_text } => clipboard
                .set_html(html, plain_text)
                .map_err(|error| AppError::Clipboard(error.to_string())),
            ClipboardContent::Image { image } => {
                let rgba = decode_png_rgba(&image.png_bytes)?;
                clipboard
                    .set_image(ImageData {
                        width: image.width,
                        height: image.height,
                        bytes: rgba.into(),
                    })
                    .map_err(|error| AppError::Clipboard(error.to_string()))
            }
            ClipboardContent::Files { bundle } => {
                let target_dir = temp_bundle_dir(transfer_id.as_deref(), &bundle.transfer_id.to_string())?;
                let file_paths = bundle
                    .files
                    .iter()
                    .map(|file| target_dir.join(safe_file_name(&file.relative_path)))
                    .collect::<Vec<_>>();
                clipboard
                    .set()
                    .file_list(&file_paths)
                    .map_err(|error| AppError::Clipboard(error.to_string()))
            }
        }
    })
    .await
    .map_err(|error| AppError::Clipboard(error.to_string()))?
}

pub async fn wait_for_change(duration: Duration) {
    #[cfg(windows)]
    {
        let _ = task::spawn_blocking(move || windows_listener::wait_for_change(duration)).await;
    }

    #[cfg(not(windows))]
    {
        tokio::time::sleep(duration).await;
    }
}

pub fn content_hash(content: &ClipboardContent) -> String {
    let bytes = serde_json::to_vec(content).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

pub fn content_preview(content: &ClipboardContent) -> String {
    match content {
        ClipboardContent::Text { text } => format!("文本: {}", truncate(text)),
        ClipboardContent::Html { plain_text, .. } => format!(
            "HTML: {}",
            truncate(plain_text.as_deref().unwrap_or("富文本"))
        ),
        ClipboardContent::Image { image } => format!("图片: {}x{}", image.width, image.height),
        ClipboardContent::Files { bundle } => {
            let first_name = bundle
                .files
                .first()
                .map(|file| file.relative_path.clone())
                .unwrap_or_else(|| "文件".to_string());
            if bundle.files.len() == 1 {
                format!("文件: {}", truncate(&first_name))
            } else {
                format!("文件: {} 等 {} 项", truncate(&first_name), bundle.files.len())
            }
        }
    }
}

pub fn content_kind_label(content: &ClipboardContent) -> &'static str {
    match content {
        ClipboardContent::Text { .. } => "文本",
        ClipboardContent::Html { .. } => "HTML",
        ClipboardContent::Image { .. } => "图片",
        ClipboardContent::Files { .. } => "文件",
    }
}

fn truncate(value: &str) -> String {
    let compact = value.replace('\n', " ");
    if compact.chars().count() > 48 {
        compact.chars().take(48).collect::<String>() + "..."
    } else {
        compact
    }
}

fn build_file_bundle(paths: Vec<PathBuf>) -> AppResult<Option<(FileBundle, Vec<LocalFileSource>)>> {
    let mut files = Vec::new();
    let mut local_files = Vec::new();
    let mut total_bytes = 0u64;
    let transfer_id = uuid::Uuid::new_v4();

    for path in paths.into_iter().take(MAX_FILE_COUNT) {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };

        let file_len = metadata.len();
        if file_len == 0 || file_len > MAX_FILE_BYTES {
            continue;
        }
        if total_bytes.saturating_add(file_len) > MAX_TOTAL_FILE_BYTES {
            break;
        }

        total_bytes += file_len;
        let relative_path = safe_bundle_name(&path);
        files.push(FileBundleFile {
            relative_path: relative_path.clone(),
            byte_len: file_len,
        });
        local_files.push(LocalFileSource {
            source_path: path,
            byte_len: file_len,
        });
    }

    if files.is_empty() {
        return Ok(None);
    }

    Ok(Some((
        FileBundle {
            transfer_id,
            files,
            total_bytes,
        },
        local_files,
    )))
}

fn safe_bundle_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "clipboard-file".to_string())
}

pub fn temp_bundle_dir(transfer_id: Option<&str>, bundle_id: &str) -> AppResult<PathBuf> {
    let key = transfer_id.unwrap_or(bundle_id);
    let dir = std::env::temp_dir().join("UniPaste").join(key);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn target_bundle_path(root: &Path, relative_path: &str) -> PathBuf {
    root.join(safe_file_name(relative_path))
}

fn encode_png_rgba(width: usize, height: usize, rgba: &[u8]) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| AppError::Clipboard(error.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| AppError::Clipboard(error.to_string()))?;
    }
    Ok(bytes)
}

fn decode_png_rgba(png_bytes: &[u8]) -> AppResult<Vec<u8>> {
    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| AppError::Clipboard(error.to_string()))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| AppError::Clipboard(error.to_string()))?;
    Ok(buffer[..info.buffer_size()].to_vec())
}

fn dispatch(
    device_id: uuid::Uuid,
    content: ClipboardContent,
    local_files: Vec<LocalFileSource>,
) -> ClipboardDispatch {
    let content_hash = content_hash(&content);
    ClipboardDispatch {
        payload: ClipboardPayload {
            message_id: uuid::Uuid::new_v4(),
            source_device_id: device_id,
            created_at_ms: crate::app_state::now_ms(),
            content_hash,
            content,
        },
        local_files,
    }
}

fn safe_file_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .map(|part| part.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "clipboard-file".to_string())
}

#[cfg(windows)]
mod windows_listener {
    use std::{
        ptr::null_mut,
        sync::{mpsc, Mutex, OnceLock},
        thread,
        time::Duration,
    };

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            AddClipboardFormatListener, CreateWindowExW, DefWindowProcW, DispatchMessageW,
            GetMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, HWND_MESSAGE, MSG,
            WM_CLIPBOARDUPDATE, WM_DESTROY, WNDCLASSW,
        },
    };

    static RECEIVER: OnceLock<Mutex<mpsc::Receiver<()>>> = OnceLock::new();
    static SENDER: OnceLock<mpsc::Sender<()>> = OnceLock::new();

    pub fn wait_for_change(duration: Duration) {
        let receiver = RECEIVER.get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            let _ = SENDER.set(tx);
            thread::spawn(run_listener);
            Mutex::new(rx)
        });
        let _ = receiver.lock().ok().and_then(|rx| rx.recv_timeout(duration).ok());
    }

    fn run_listener() {
        unsafe {
            let class_name = wide("UniPasteClipboardListener");
            let hinstance = GetModuleHandleW(null_mut());
            let wnd = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: hinstance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            let _ = RegisterClassW(&wnd);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                0,
                hinstance,
                null_mut(),
            );
            if hwnd != 0 {
                let _ = AddClipboardFormatListener(hwnd);
                let mut message = MSG::default();
                while GetMessageW(&mut message, 0, 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_CLIPBOARDUPDATE => {
                if let Some(sender) = SENDER.get() {
                    let _ = sender.send(());
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}
