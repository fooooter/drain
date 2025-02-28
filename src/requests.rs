use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::str::FromStr;

use bstr::ByteSlice;
use libloading::Error as LibError;
use mime_guess::{mime, Mime};
use regex::Regex;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::config::CONFIG;
use crate::error::ServerError;
use crate::pages::index_of;
use crate::util::{
    endpoint, send_response, rte_wrapper, ResourceType::{Dynamic, Static}, ETAGS,
};
use drain_common::{cookies::SetCookie, RequestBody, RequestData::*};

#[derive(Debug)]
pub enum Request {
    Get {
        resource: String,
        params: Option<HashMap<String, String>>,
        headers: HashMap<String, String>,
    },
    Head {
        resource: String,
        params: Option<HashMap<String, String>>,
        headers: HashMap<String, String>,
    },
    Post {
        resource: String,
        params: Option<HashMap<String, String>>,
        headers: HashMap<String, String>,
        data: Option<RequestBody>,
    },
    Put {
        resource: String,
        headers: HashMap<String, String>,
        data: Option<RequestBody>,
    },
    Delete,
    Connect,
    Options {
        resource: String,
        headers: HashMap<String, String>,
    },
    Trace(Vec<u8>),
    Patch {
        resource: String,
        headers: HashMap<String, String>,
        data: Option<RequestBody>,
    },
}

impl Request {
    pub fn parse_from_string(request_string: &str) -> Result<Self, ServerError> {
        let method_regex = Regex::new(
            r"^(?P<method>GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH)\s+(?P<path>[^?\s]*)"
        ).unwrap();

        let captures = method_regex
            .captures(request_string)
            .ok_or(ServerError::InvalidRequest)?;

        let method = captures["method"].to_uppercase();
        let mut resource = captures["path"].trim().to_string();

        let (resource_path, params) = Self::parse_query_params(&resource)?;
        resource = resource_path;

        let headers = Self::parse_headers(request_string)?;

        Self::build_request(method, resource, params, headers, request_string)
    }

    fn parse_query_params(resource: &str) -> Result<(String, Option<HashMap<String, String>>), ServerError> {
        let mut params = HashMap::new();
        let (path, query) = resource.split_once('?').unwrap_or((resource, ""));

        for pair in query.split('&').filter(|s| !s.is_empty()) {
            let (key, value) = pair.split_once('=').ok_or(ServerError::InvalidRequest)?;
            if params.insert(key.to_string(), value.to_string()).is_some() {
                return Err(ServerError::InvalidRequest);
            }
        }

        Ok((path.to_string(), (!params.is_empty()).then_some(params)))
    }

    fn parse_headers(request_string: &str) -> Result<HashMap<String, String>, ServerError> {
        let header_regex = Regex::new(r"(?m)^([[:alnum:]-_]+):\s*(.*)").unwrap();
        let mut headers = HashMap::new();

        for cap in header_regex.captures_iter(request_string) {
            let key = cap[1].trim().to_lowercase();
            let value = cap[2].trim().to_string();
            headers.insert(key, value);
        }

        Ok(headers)
    }

    fn build_request(
        method: String,
        resource: String,
        params: Option<HashMap<String, String>>,
        headers: HashMap<String, String>,
        request_string: &str,
    ) -> Result<Self, ServerError> {
        match method.as_str() {
            "TRACE" => Ok(Self::Trace(request_string.as_bytes().to_vec())),
            "GET" => Ok(Self::Get { resource, params, headers }),
            "HEAD" => Ok(Self::Head { resource, params, headers }),
            "POST" => Ok(Self::Post {
                resource,
                params,
                headers,
                data: None,
            }),
            "PUT" => Ok(Self::Put {
                resource,
                headers,
                data: None,
            }),
            "DELETE" => Ok(Self::Delete),
            "CONNECT" => Ok(Self::Connect),
            "OPTIONS" => Ok(Self::Options { resource, headers }),
            "PATCH" => Ok(Self::Patch {
                resource,
                headers,
                data: None,
            }),
            _ => Err(ServerError::InvalidRequest),
        }
    }
}

async fn handle_common<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
    headers: &HashMap<String, String>,
    resource: &str,
    request_data: RequestData<'_>,
) -> Result<(), Box<dyn Error>> {
    let resource = normalize_resource(resource)?;
    check_access_control(&mut stream, &resource).await?;

    let (resource, is_dir) = resolve_resource_path(&resource).await?;
    if is_dir {
        return index_of(&mut stream, &resource, false, headers).await;
    }

    if is_endpoint(&resource) {
        return handle_endpoint_request(stream, request_data, headers, &resource).await;
    }

    serve_file(stream, &resource, headers).await
}

pub async fn handle_get<T: AsyncRead + AsyncWrite + Unpin>(
    stream: T,
    headers: &HashMap<String, String>,
    resource: String,
    params: &Option<HashMap<String, String>>,
) -> Result<(), Box<dyn Error>> {
    handle_common(stream, headers, &resource, Get(params)).await
}

pub async fn handle_head<T: AsyncRead + AsyncWrite + Unpin>(
    stream: T,
    headers: &HashMap<String, String>,
    resource: String,
    params: &Option<HashMap<String, String>>,
) -> Result<(), Box<dyn Error>> {
    handle_common(stream, headers, &resource, Head(params)).await
}

pub async fn handle_post<T: AsyncRead + AsyncWrite + Unpin>(
    stream: T,
    headers: &HashMap<String, String>,
    resource: String,
    data: &Option<RequestBody>,
    params: &Option<HashMap<String, String>>,
) -> Result<(), Box<dyn Error>> {
    handle_common(stream, headers, &resource, Post { data, params }).await
}

async fn check_access_control<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    resource: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(resource) {
            let deny_action = access_control.deny_action;
            let endpoint_name = if deny_action == 404 { "not_found" } else { "forbidden" };
            handle_denied_request(stream, deny_action, endpoint_name).await?;
        }
    }
    Ok(())
}

async fn handle_denied_request<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    status: u16,
    endpoint_name: &str,
) -> Result<(), Box<dyn Error>> {
    let mut response_headers = HashMap::new();
    let mut set_cookie = HashMap::new();

    let content = endpoint(
        endpoint_name,
        stream,
        Get(&None),
        &HashMap::new(),
        &mut response_headers,
        &mut set_cookie,
    )
    .await;

    process_and_send_response(stream, status, content, response_headers, set_cookie).await
}

async fn process_and_send_response<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    status: u16,
    content: Result<Option<Vec<u8>>, LibError>,
    mut response_headers: HashMap<String, String>,
    set_cookie: HashMap<String, SetCookie>,
) -> Result<(), Box<dyn Error>> {
    let content_type = response_headers.get("content-type");
    let (content, mime_info) = match (content, content_type) {
        (Ok(Some(c)), Some(ct)) => {
            match Mime::from_str(ct) {
                Ok(mime) => (Some(c), Some((mime.to_string(), mime.type_().to_string()))),
                Err(_) => {
                    response_headers.remove("content-type");
                    (None, None)
                }
            }
        }
        _ => (None, None),
    };

    if let Some((mime_type, general_type)) = mime_info {
        apply_content_encoding(&mut response_headers, content.as_ref().unwrap(), &mime_type, &general_type);
    }

    send_response(
        stream,
        status,
        Some(response_headers),
        content,
        Some(set_cookie),
        Some(Dynamic),
    )
    .await
}

async fn resolve_resource_path(resource: &str) -> Result<(String, bool), Box<dyn Error>> {
    let path = PathBuf::from(&CONFIG.document_root).join(resource);
    let is_dir = path.is_dir();

    if is_dir {
        let index_files = ["index.html", "index"];
        for file in &index_files {
            let index_path = path.join(file);
            if index_path.exists() {
                return Ok((format!("{}/{}", resource, file), false));
            }
        }
    }

    Ok((resource.to_string(), is_dir))
}

async fn serve_file<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
    resource: &str,
    headers: &HashMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(&CONFIG.document_root).join(resource);
    match File::open(&path).await {
        Ok(mut file) => {
            let mut content = Vec::new();
            rte_wrapper(&mut file, &mut content, &mut stream).await;

            let mime = mime_guess::from_path(&path)
                .first()
                .unwrap_or_else(|| if content.is_utf8() {
                    mime::TEXT_PLAIN
                } else {
                    mime::APPLICATION_OCTET_STREAM
                });

            send_file_response(&mut stream, content, &mime, headers).await
        }
        Err(_) => handle_not_found(stream).await,
    }
}

async fn send_file_response<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    content: Vec<u8>,
    mime: &Mime,
    headers: &HashMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let mut response_headers = HashMap::new();
    response_headers.insert("Content-Type".into(), mime.to_string());

    apply_content_encoding(
        &mut response_headers,
        &content,
        &mime.to_string(),
        &mime.type_().to_string(),
    );

    check_cache_headers(stream, headers, &mut response_headers, &content).await?;

    send_response(
        stream,
        200,
        Some(response_headers),
        Some(content),
        None,
        Some(Static),
    )
    .await
}

async fn check_cache_headers<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    headers: &HashMap<String, String>,
    response_headers: &mut HashMap<String, String>,
    content: &[u8],
) -> Result<(), Box<dyn Error>> {
    if let Some(if_none_match) = headers.get("if-none-match") {
        let etags = ETAGS.lock().await;
        for etag in if_none_match.split(',').filter_map(|e| {
            e.trim_matches(|c: char| c.is_whitespace() || c == '"').to_string()
        }) {
            if etags.contains(&etag) {
                response_headers.insert("ETag".into(), etag);
                response_headers.insert(
                    "Cache-Control".into(),
                    format!("max-age={}", CONFIG.cache_max_age),
                );
                send_response(stream, 304, Some(response_headers.clone()), None, None, None).await?;
                return Ok(());
            }
        }
    }
    Ok(())
}

async fn handle_not_found<T: AsyncRead + AsyncWrite + Unpin>(mut stream: T) -> Result<(), Box<dyn Error>> {
    let mut response_headers = HashMap::new();
    let mut set_cookie = HashMap::new();
    
    let content = endpoint(
        "not_found",
        &mut stream,
        Get(&None),
        &HashMap::new(),
        &mut response_headers,
        &mut set_cookie,
    )
    .await;

    process_and_send_response(&mut stream, 404, content, response_headers, set_cookie).await
}

pub async fn handle_options<T: AsyncRead + AsyncWrite + Unpin>(mut stream: T) -> Result<(), Box<dyn Error>> {
    let mut methods = vec!["GET", "HEAD", "POST", "OPTIONS"];
    if CONFIG.enable_trace {
        methods.push("TRACE");
    }

    let response_headers = HashMap::from([("Allow".into(), methods.join(", "))]);
    send_response(&mut stream, 204, Some(response_headers), None, None, None).await
}

fn normalize_resource(resource: &str) -> Result<String, Box<dyn Error>> {
    Ok(resource.trim_start_matches('/').to_string())
}

fn apply_content_encoding(
    headers: &mut HashMap<String, String>,
    content: &[u8],
    mime_type: &str,
    general_type: &str,
) {
    if let Some(encoding) = CONFIG.get_response_encoding(content, mime_type, general_type, &HashMap::new()) {
        headers.insert("Content-Encoding".into(), encoding.to_string());
        headers.insert("Vary".into(), "Accept-Encoding".into());
    }
}

fn is_endpoint(resource: &str) -> bool {
    CONFIG.endpoints.as_ref().map_or(false, |e| e.contains(resource))
}