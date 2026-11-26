use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::Mutex,
};

use tauri::{
    http::{header, Method, Request, Response, StatusCode},
    AppHandle, Manager, Runtime, State, UriSchemeResponder,
};

use super::Storage;

const MAX_RESPONSE_SIZE: u64 = 2 * 1024 * 1024;

pub fn handle_preview_request<R: Runtime>(
    app: AppHandle<R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let response = match build_preview_response(&app, &request) {
        Ok(response) => response,
        Err(status) => empty_response(status),
    };

    let _ = responder.respond(response);
}

fn build_preview_response<R: Runtime>(
    app: &AppHandle<R>,
    request: &Request<Vec<u8>>,
) -> Result<Response<Vec<u8>>, StatusCode> {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    let segments: Vec<&str> = request.uri().path().trim_matches('/').split('/').collect();

    if segments.len() != 2
        || segments.iter().any(|segment| {
            segment.is_empty()
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let chat_id = segments[0];
    let reference_id = segments[1];

    let state: State<'_, Mutex<Storage>> = app.state();
    let (path, file_name, mime_type, file_size) = {
        let storage = state
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let preview = storage
            .get_preview_blob(chat_id, reference_id)
            .map_err(|_| StatusCode::NOT_FOUND)?;

        let path = storage
            .resolve_blob_path(&preview.relative_path)
            .map_err(|_| StatusCode::NOT_FOUND)?;

        (
            path,
            preview.file_name,
            safe_mime_type(&preview.mime_type).to_string(),
            preview.original_size as u64,
        )
    };

    let range_header = request.headers().get(header::RANGE);
    let (start, end, status) = match range_header {
        Some(value) => {
            let value = value
                .to_str()
                .map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?;
            let (start, end) = parse_range(value, file_size)?;
            (start, end, StatusCode::PARTIAL_CONTENT)
        }
        None => {
            if file_size > MAX_RESPONSE_SIZE {
                return Err(StatusCode::RANGE_NOT_SATISFIABLE);
            }
            (0, file_size.saturating_sub(1), StatusCode::OK)
        }
    };

    let length = if file_size == 0 { 0 } else { end - start + 1 };

    if length > MAX_RESPONSE_SIZE {
        return Err(StatusCode::RANGE_NOT_SATISFIABLE);
    }

    let mut body = Vec::new();

    if request.method() == Method::GET && length > 0 {
        let mut file = File::open(path).map_err(|_| StatusCode::NOT_FOUND)?;
        file.seek(SeekFrom::Start(start))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        body.resize(length as usize, 0);
        file.read_exact(&mut body)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    let disposition = format!("inline; filename=\"{}\"", safe_filename(&file_name));

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::CONTENT_LENGTH, length.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_DISPOSITION, disposition)
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "default-src 'none'; sandbox");

    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{file_size}"),
        );
    }

    builder
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn parse_range(value: &str, size: u64) -> Result<(u64, u64), StatusCode> {
    if size == 0 {
        return Err(StatusCode::RANGE_NOT_SATISFIABLE);
    }

    let value = value
        .strip_prefix("bytes=")
        .ok_or(StatusCode::RANGE_NOT_SATISFIABLE)?;

    if value.contains(',') {
        return Err(StatusCode::RANGE_NOT_SATISFIABLE);
    }

    let (start, end) = value
        .split_once('-')
        .ok_or(StatusCode::RANGE_NOT_SATISFIABLE)?;

    let (start, end) = if start.is_empty() {
        let suffix: u64 = end.parse().map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?;

        if suffix == 0 {
            return Err(StatusCode::RANGE_NOT_SATISFIABLE);
        }

        (size.saturating_sub(suffix), size - 1)
    } else {
        let start: u64 = start
            .parse()
            .map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?;

        let end = if end.is_empty() {
            size - 1
        } else {
            end.parse::<u64>()
                .map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?
                .min(size - 1)
        };

        (start, end)
    };

    if start >= size || end < start || end - start + 1 > MAX_RESPONSE_SIZE {
        return Err(StatusCode::RANGE_NOT_SATISFIABLE);
    }

    Ok((start, end))
}

fn safe_mime_type(mime: &str) -> &str {
    match mime {
        "application/pdf" | "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        | "image/bmp" | "text/plain" | "text/markdown" => mime,
        _ => "application/octet-stream",
    }
}

fn safe_filename(name: &str) -> String {
    name.chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ')
        })
        .take(120)
        .collect::<String>()
        .replace('"', "")
}

fn empty_response(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_LENGTH, "0")
        .header("X-Content-Type-Options", "nosniff")
        .body(Vec::new())
        .unwrap_or_else(|_| Response::new(Vec::new()))
}
