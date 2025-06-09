use std::collections::HashMap;
use std::error::Error;
use std::net::IpAddr;
use std::path::Path;
use std::process::Stdio;
use std::str::FromStr;
use bstr::ByteSlice;
use drain_common::cookies::SetCookie;
use drain_common::RequestData::Get;
use mime_guess::Mime;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, Command};
use crate::config::CONFIG;
use crate::endpoints::{endpoint, ENDPOINT_LIBRARY};
use crate::error::ServerError;
use crate::util::ResourceType::Dynamic;
use crate::util::{send_response, CHROOT};

pub struct CGIData {
    pub data: Vec<u8>,
    pub content_type: String
}

pub enum CGIStatus {
    Available,
    Unavailable,
    Denied
}

pub async fn handle_cgi<T>(stream: &mut T,
                           headers: &HashMap<String, String>,
                           resource: &String,
                           request_method: &str,
                           query_string: String,
                           cgi_data: Option<CGIData>,
                           local_ip: &IpAddr,
                           remote_ip: &IpAddr,
                           remote_port: &u16,
                           https: bool) -> Result<CGIStatus, Box<dyn Error + Send + Sync>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let mut response_headers: HashMap<String, String> = HashMap::new();
    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(&String::from((&resource).trim_start_matches('/'))) {
            let mut deny_action = access_control.deny_action;
            if let Some(library) = &*ENDPOINT_LIBRARY {
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint(
                    if deny_action == 404 { "not_found" } else { "forbidden" },
                    stream,
                    Get(&None),
                    headers,
                    &mut response_headers,
                    &mut set_cookie,
                    &mut deny_action,
                    local_ip,
                    remote_ip,
                    remote_port,
                    library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        if let Err(_) = send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await {
                            return Err(Box::new(ServerError::BadGateway));
                        }
                        return Ok(CGIStatus::Denied);
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if let Err(_) = send_response(stream, deny_action, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await {
                        return Err(Box::new(ServerError::BadGateway));
                    }
                    return Ok(CGIStatus::Denied);
                }
                if let Err(_) = send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await {
                    return Err(Box::new(ServerError::BadGateway));
                }
                return Ok(CGIStatus::Denied);
            }
            if let Err(_) = send_response(stream, deny_action, Some(response_headers), None, None, None).await {
                return Err(Box::new(ServerError::BadGateway));
            }
            return Ok(CGIStatus::Denied);
        }
    }

    let gateway_interface: String = String::from("CGI/1.1");
    let server_addr = local_ip.to_string();
    let server_name = &CONFIG.bind_host;
    let server_port = CONFIG.bind_port.to_string();
    let server_protocol = String::from("HTTP/1.1");
    let server_software = format!("Drain {}", env!("CARGO_PKG_VERSION"));
    #[cfg(target_family = "unix")]
    let document_root = if *&*CHROOT {&String::from("")} else {&CONFIG.document_root};
    #[cfg(not(target_family = "unix"))]
    let document_root = &CONFIG.document_root;
    let content_length: String;
    let request_uri = resource;

    let path_split: Vec<&str> = resource.split("/").collect();
    let mut script_filename = String::from(document_root);
    let mut file_pos = 1;

    while !Path::is_file(Path::new(&script_filename)) && file_pos < path_split.len() {
        script_filename.push_str(&*format!("/{}", path_split[file_pos]));
        file_pos += 1;
    }

    let script_name = &path_split[file_pos - 1];
    let mut path_info = String::from("");
    for i in file_pos..path_split.len() {
        path_info.push_str(&*format!("/{}", path_split[i]));
    }

    let remote_addr = remote_ip.to_string();
    let remote_port = remote_port.to_string();

    let mut envs: HashMap<String, String> = HashMap::from([
        (String::from("GATEWAY_INTERFACE"), gateway_interface),
        (String::from("SERVER_ADDR"), server_addr),
        (String::from("SERVER_NAME"), server_name.clone()),
        (String::from("SERVER_PORT"), server_port),
        (String::from("SERVER_PROTOCOL"), server_protocol),
        (String::from("SERVER_SOFTWARE"), server_software),
        (String::from("DOCUMENT_ROOT"), document_root.clone()),
        (String::from("REQUEST_URI"), request_uri.clone()),
        (String::from("REQUEST_METHOD"), String::from(request_method)),
        (String::from("QUERY_STRING"), query_string),
        (String::from("HTTPS"), if https {String::from("1")} else {String::from("")}),
        (String::from("REMOTE_ADDR"), remote_addr),
        (String::from("REMOTE_PORT"), remote_port),
        (String::from("SCRIPT_NAME"), script_name.to_string()),
        (String::from("SCRIPT_FILENAME"), script_filename.clone()),
        (String::from("REDIRECT_STATUS"), String::from("1")),
        (String::from("PATH_INFO"), path_info)
    ]);

    envs.extend(headers.iter().map(|(k, v)| (format!("HTTP_{}", k.replace('-', "_")).to_uppercase(), v.clone())));
    envs.remove("HTTP_CONTENT_TYPE");
    envs.remove("HTTP_CONTENT_LENGTH");

    let Some(cgi) = &CONFIG.cgi else {
        return Err(Box::new(ServerError::BadGateway));
    };

    let mut cgi_command = Command::new(&cgi.cgi_server);
    let mut cgi_process: Child;

    if let Some(cgi_data) = cgi_data {
        content_length = cgi_data.data.len().to_string();
        let data = cgi_data.data;
        let content_type = cgi_data.content_type;

        envs.insert(String::from("CONTENT_TYPE"), content_type);
        envs.insert(String::from("CONTENT_LENGTH"), content_length);

        cgi_process = cgi_command
            .envs(&envs)
            .arg(&script_filename)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let Some(ref mut stdin) = cgi_process.stdin else {
            return Err(Box::new(ServerError::BadGateway));
        };

        stdin.write_all(&*data).await?;
    } else {
        cgi_process = cgi_command
            .envs(&envs)
            .arg(&script_filename)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
    }

    let output = cgi_process.wait_with_output().await?;

    match (output.stderr.is_empty(), output.status.success()) {
        (true, false) => return Ok(CGIStatus::Unavailable),
        (false, false) => {
            if CONFIG.be_verbose {
                eprintln!("[handle_cgi():{}] Standard error message received while executing {script_filename}:\n{}", line!(), String::from_utf8_lossy(&*output.stderr));
            }
            return Err(Box::new(ServerError::BadGateway));
        },
        (false, true) if CONFIG.be_verbose =>
            eprintln!("[handle_cgi():{}] Standard error message received while executing {script_filename}:\n{}", line!(), String::from_utf8_lossy(&*output.stderr)),
        _ => {}
    }

    let Some((headers, content)) = output.stdout.split_once_str("\r\n\r\n") else {
        return Err(Box::new(ServerError::BadGateway));
    };

    let headers_iter = headers.split_str("\r\n");
    for header_field in headers_iter {
        let Some((name, value)) = header_field.split_once_str(":") else {
            return Err(Box::new(ServerError::BadGateway));
        };

        response_headers.insert(String::from_utf8_lossy(name).trim().to_lowercase(), String::from(String::from_utf8_lossy(value).trim()));
    }

    let status = match response_headers.get("status") {
        Some(status_raw) => {
            if let Ok(s) = (&status_raw.as_str()[0..3]).parse::<u16>() {
                s
            } else {
                return Err(Box::new(ServerError::BadGateway));
            }
        },
        None => 200
    };
    response_headers.remove("status");

    if let Err(_) = send_response(stream,
                                  if response_headers.contains_key("location") {302} else {status},
                                  Some(response_headers),
                                  if content.is_empty() {None} else {Some(content.to_vec())},
                                  None,
                                  Some(Dynamic)).await {
        return Err(Box::new(ServerError::BadGateway));
    }

    Ok(CGIStatus::Available)
}