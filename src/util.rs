use std::collections::HashMap;
use std::error::Error;
use std::io::Read;
use chrono::Utc;
use brotli::{BrotliCompress, BrotliDecompress};
use brotli::enc::BrotliEncoderParams;
use flate2::Compression;
use flate2::read::{GzDecoder, GzEncoder};
use libloading::{Library, Error as LibError};
use openssl::hash::{hash, MessageDigest};
use openssl::base64;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, BufReader};
use tokio::io::AsyncWriteExt;
use bstr::ByteSlice;
use bytes::BytesMut;
use openssl::error::ErrorStack;
use drain_common::cookies::{SetCookie, SameSite};
use drain_common::{FormDataValue, RequestBody, RequestData};
use drain_common::RequestBody::{FormData, XWWWFormUrlEncoded};
use crate::pages::internal_server_error::internal_server_error;
use crate::config::Config;
use crate::requests::Request;
use crate::error::*;

type Page = fn(RequestData, &HashMap<String, String>, &mut HashMap<String, String>, &mut HashMap<String, SetCookie>) -> Option<Vec<u8>>;

pub async fn send_response<T>(stream: &mut T,
                              config: Option<&Config>,
                              status: u16,
                              mut local_response_headers: Option<HashMap<String, String>>,
                              content: Option<Vec<u8>>,
                              set_cookie: Option<HashMap<String, SetCookie>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let mut response = String::new();
    let status_text = match status {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => return Err(Box::new(ServerError::InvalidStatusCode(status)))
    };
    let status_line = format!("HTTP/1.1 {status} {status_text}\r\n");
    response.push_str(&*status_line);

    let date = get_current_date();
    let date_header = format!("Date: {}\r\n", date);
    response.push_str(&*date_header);

    let global_response_headers = if let Some(c) = config {
        c.global_response_headers.clone()
    } else {
        HashMap::from([(String::from("Connection"), String::from("close"))])
    };

    if let Some(set_cookie) = set_cookie {
        if !set_cookie.is_empty() {
            for (k, v) in set_cookie {
                response.push_str(&format!("Set-Cookie: {}={}", k, v.value));
                if let Some(domain) = &v.domain {
                    response.push_str(&format!("; Domain={}", domain));
                }
                if let Some(expires) = &v.expires {
                    response.push_str(&format!("; Expires={}", expires));
                }
                if v.httponly {
                    response.push_str("; HttpOnly");
                }
                if let Some(max_age) = &v.max_age {
                    response.push_str(&format!("; Max-Age={}", max_age));
                }
                if v.partitioned {
                    response.push_str("; Partitioned");
                }
                if let Some(path) = &v.path {
                    response.push_str(&format!("; Path={}", path));
                }
                if v.secure {
                    response.push_str("Secure");
                }
                if let Some(samesite) = &v.samesite {
                    match samesite {
                        SameSite::Strict => {
                            response.push_str("; SameSite=Strict");
                        },
                        SameSite::Lax => {
                            response.push_str("; SameSite=Lax");
                        },
                        SameSite::None => {
                            response.push_str("; SameSite=None");
                        }
                    }
                }
                response.push_str("\r\n");
            }
        }
    }

    let mut response_bytes: Vec<u8>;

    match (&mut local_response_headers, content) {
        (Some(ref mut h), Some(c)) => {
            h.extend(global_response_headers);

            for (k, v) in &mut *h {
                response.push_str(&*format!("{k}: {v}\r\n"));
            }

            let mut content_trim = c.trim_ascii();
            let mut content_prepared: Vec<u8> = Vec::new();

            if let Some(encoding) = h.get("Content-Encoding") {
                if encoding.eq("gzip") {
                    if let Err(e) = GzEncoder::new(content_trim, Compression::default()).read_to_end(&mut content_prepared) {
                        eprintln!("[send_response():{}] An error occurred while compressing the content of a response using GZIP:\n{e}\n\
                                    Attempting to send uncompressed data...", line!());
                        content_prepared = Vec::from(c.trim_ascii());
                    }
                } else if encoding.eq("br") {
                    if let Err(e) = BrotliCompress(&mut content_trim, &mut content_prepared, &BrotliEncoderParams::default()) {
                        eprintln!("[send_response():{}] An error occurred while compressing the content of a response using GZIP:\n{e}\n\
                                    Attempting to send uncompressed data...", line!());
                        content_prepared = Vec::from(c.trim_ascii());
                    }
                } else {
                    content_prepared = Vec::from(c.trim_ascii());
                }
            } else {
                content_prepared = Vec::from(c.trim_ascii());
            }

            let content_length_header = format!("Content-Length: {}\r\n", content_prepared.len());
            response.push_str(&*content_length_header);

            match generate_etag(&*content_prepared) {
                Ok(etag) => {
                    let etag_header = format!("ETag: {etag}\r\n\r\n");
                    response.push_str(&*etag_header);
                },
                Err(e) => {
                    eprintln!("[send_response():{}] An error occurred while generating an ETag:\n{e}\n\
                                Continuing without ETag...", line!());
                }
            }

            response_bytes = Vec::from(response);
            for x in content_prepared {
                response_bytes.push(x);
            }
        },
        (None, Some(c)) => {
            let content_trim = c.trim_ascii_start();
            let content_length_header = format!("Content-Length: {}\r\n", content_trim.len());
            response.push_str(&*content_length_header);

            match generate_etag(content_trim) {
                Ok(etag) => {
                    let etag_header = format!("ETag: {etag}\r\n\r\n");
                    response.push_str(&*etag_header);
                },
                Err(e) => {
                    eprintln!("[send_response():{}] An error occurred while generating an ETag:\n{e}\n\
                                Continuing without ETag...", line!());
                }
            }

            response_bytes = Vec::from(response);
            for x in content_trim {
                response_bytes.push(*x);
            }
        },
        (Some(ref mut h), None) => {
            h.extend(global_response_headers);

            for (k, v) in h {
                response.push_str(&*format!("{k}: {v}\r\n"));
            }

            response.push_str("\r\n");
            response_bytes = Vec::from(response);
        },
        (None, None) => {
            response.push_str("\r\n");
            response_bytes = Vec::from(response);
        }
    }

    if let Err(e1) = stream.write_all(&*response_bytes).await {
        eprintln!("[send_response():{}] An error occurred while writing a response to a client:\n{e1}\n\
                    Attempting to close connection...", line!());
        if let Err(e2) = stream.shutdown().await {
            eprintln!("[receive_request():{}] FAILED. Error information:\n{e2}", line!());
        }
        panic!("Unrecoverable error occurred while handling connection.");
    }

    if let Err(e) = stream.flush().await {
        eprintln!("[send_response():{}] An error occurred while flushing the output stream:\n{e}", line!());
        return Err(Box::new(e));
    }

    Ok(())
}

pub async fn receive_request<T>(stream: &mut T, config: &Config) -> Result<Request, ServerError>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let mut reader = BufReader::new(&mut *stream);
    let mut request_string = String::new();

    loop {
        match reader.read_line(&mut request_string).await {
            Ok(l) => {
                if l == 2 {
                    break;
                }
            },
            Err(e1) => {
                eprintln!("[receive_request():{}] An error occurred while reading a request from a client.\n\
                            Error information:\n{e1}\n\
                            Attempting to close connection...", line!());
                if let Err(e2) = stream.shutdown().await {
                    eprintln!("[receive_request():{}] FAILED. Error information:\n{e2}", line!());
                }
                panic!("Unrecoverable error occurred while handling connection.");
            }
        };
    }

    let mut request = Request::parse_from_string(&request_string)?;

    if let  Request::Post {ref mut data, headers, ..} |
            Request::Put {ref mut data, headers, ..} |
            Request::Patch {ref mut data, headers, ..} = &mut request {
        let mut buffer = BytesMut::with_capacity(
            if let Ok(l) = headers.get("content-length").unwrap_or(&String::from("0")).parse::<usize>() {
                if l != 0 {
                    l
                } else {
                    return Err(ServerError::InvalidRequest);
                }
            } else {
                return Err(ServerError::InvalidRequest);
            }
        );

        if let Err(e1) = reader.read_buf(&mut buffer).await {
            eprintln!("[receive_request():{}] An error occurred while reading a request from a client.\n\
                        Error information:\n{e1}\n\
                        Attempting to close connection...", line!());
            if let Err(e2) = stream.shutdown().await {
                eprintln!("[receive_request():{}] FAILED. Error information:\n{e2}", line!());
            }
            panic!("Unrecoverable error occurred while handling connection.");
        }

        let mut payload: Vec<u8> = Vec::new();

        match (headers.get("content-encoding"), config.get_supported_encodings()) {
            (Some(content_encoding), Some(supported_encodings))
            if supported_encodings.contains(content_encoding) => {
                if content_encoding.eq("gzip") {
                    if let Err(e) = GzDecoder::new(&*buffer).read_to_end(&mut payload) {
                        eprintln!("[receive_request():{}] An error occurred while decompressing the request body using GZIP:\n{e}\n\
                                    Sending 406 status to the client...", line!());

                        return Err(ServerError::DecompressionError(e));
                    }
                } else if content_encoding.eq("br") {
                    if let Err(e) = BrotliDecompress(&mut &*buffer, &mut payload) {
                        eprintln!("[receive_request():{}] An error occurred while decompressing the request body using GZIP:\n{e}\n\
                                    Sending 406 status to the client...", line!());

                        return Err(ServerError::DecompressionError(e));
                    }
                } else {
                    return Err(ServerError::UnsupportedEncoding);
                }
            }
            (Some(content_encoding), Some(supported_encodings))
            if !supported_encodings.contains(content_encoding) => {
                return Err(ServerError::UnsupportedEncoding);
            },
            (Some(_), None) => {
                return Err(ServerError::UnsupportedEncoding);
            },
            _ => {
                payload = buffer.to_vec();
            }
        }

        let body: RequestBody;

        match headers.get("content-type") {
            Some(content_type) if content_type.starts_with("application/x-www-form-urlencoded") => {
                let x_www_urlencoded_raw = String::from(String::from_utf8_lossy(&payload));
                let mut body_hm: HashMap<String, String> = HashMap::new();
                for kv in x_www_urlencoded_raw.split('&') {
                    let Some(kv_split) = kv.split_once('=') else {
                        return Err(ServerError::MalformedPayload);
                    };

                    let (Ok(name_decoded), Ok(value_decoded)) = (urlencoding::decode(kv_split.0), urlencoding::decode(kv_split.1)) else {
                        return Err(ServerError::MalformedPayload);
                    };

                    if let Some(_) = &body_hm.insert(name_decoded.into_owned(), value_decoded.into_owned()) {
                        return Err(ServerError::MalformedPayload);
                    }
                }
                body = XWWWFormUrlEncoded(body_hm);
            },
            Some(content_type) => {
                let Some((content_type, boundary_raw)) = content_type.split_once(';') else {
                    return Err(ServerError::MalformedPayload);
                };

                if !content_type.trim_end().starts_with("multipart/form-data") {
                    return Err(ServerError::UnsupportedMediaType);
                }

                let Some((_, bound)) = boundary_raw.trim_end_matches(';').split_once('=') else {
                    return Err(ServerError::MalformedPayload);
                };
                let bound = bound.trim_matches(|y| y == '"');

                let mut body_hm: HashMap<String, FormDataValue> = HashMap::new();

                for field in payload.split_str(&*format!("--{bound}")).skip(1) {
                    if field.trim_ascii().eq(&[45, 45]) {
                        break;
                    }

                    let mut field_lines = field.split_str("\r\n").skip(1);
                    let (Some(content_disp), Some(blank_line), Some(field_data)) = (field_lines.next(), field_lines.next(), field_lines.next()) else {
                        return Err(ServerError::MalformedPayload);
                    };

                    let Some((_, content_disp_val)) = content_disp.split_once_str(":") else {
                        return Err(ServerError::MalformedPayload);
                    };

                    let mut content_disp_val = content_disp_val.split_str(";");
                    let (Some(form_data), Some(name)) = (content_disp_val.next(), content_disp_val.next()) else {
                        return Err(ServerError::MalformedPayload);
                    };

                    if !String::from_utf8_lossy(form_data).trim_start().eq("form-data") || !blank_line.trim_ascii().is_empty() {
                        return Err(ServerError::MalformedPayload);
                    }

                    let Some((_, name)) = name.split_once_str("=") else {
                        return Err(ServerError::MalformedPayload);
                    };

                    let v = FormDataValue {
                        filename: if let Some(filename) = content_disp_val.next() {
                            let Some((_, filename)) = filename.split_once_str("=") else {
                                return Err(ServerError::MalformedPayload);
                            };

                            Some(String::from(String::from_utf8_lossy(filename).trim_matches('"')))
                        } else {
                            None
                        },
                        value: Vec::from(field_data)
                    };

                    body_hm.insert(String::from(String::from_utf8_lossy(name).trim_matches('"')), v);
                }
                body = FormData(body_hm);
            },
            _ => {
                return Err(ServerError::UnsupportedMediaType);
            }
        }

        *data = Some(body);
    }
    Ok(request)
}

pub fn generate_etag(content: &[u8]) -> Result<String, ErrorStack>  {
    Ok(base64::encode_block(&*hash(MessageDigest::md5(), content)?))
}

pub async fn rte_wrapper<T>(f: &mut File, buf: &mut Vec<u8>, stream: &mut T)
where
    T: AsyncRead + AsyncWrite + Unpin
{
    if let Err(e1) = f.read_to_end(buf).await {
        eprintln!("[rte_wrapper():{}] An error occurred after an attempt to read from a file: {:?}.\n\
                   Error information:\n{e1}\n\
                   Attempting to send Internal Server Error page to the client...", line!(), f);
        if let Err(e2) = internal_server_error(stream).await {
            eprintln!("[rte_wrapper():{}] FAILED. Error information: {e2}", line!());
        }
        eprintln!("Attempting to close connection...");
        if let Err(e2) = stream.shutdown().await {
            eprintln!("[rte_wrapper():{}] FAILED. Error information:\n{e2}", line!());
        }
        panic!("Unrecoverable error occurred while handling connection.");
    }
}

pub fn get_current_date() -> String {
    let dt = Utc::now();
    let dt_formatted = dt.format("%a, %e %b %Y %T GMT");
    dt_formatted.to_string()
}

pub fn page<'a>(page: &str,
                request_data: RequestData<'a>,
                request_headers: &HashMap<String, String>,
                mut response_headers: &mut HashMap<String, String>,
                mut set_cookie: &mut HashMap<String, SetCookie>,
                config: &Config) -> Result<Option<Vec<u8>>, LibError>
{
    unsafe {
        let page_symbol = String::from(page).replace("/", "::");
        let lib = Library::new(format!("{}/{}", &config.server_root, &config.dynamic_pages_library))?;
        let p = lib.get::<Page>(page_symbol.as_bytes())?;
        let content = p(request_data, &request_headers, &mut response_headers, &mut set_cookie);
        lib.close()?;

        Ok(content)
    }
}