mod requests;
mod util;
mod pages;
mod config;
mod error;
#[cfg(feature = "cgi")]
mod cgi;
mod ssl;
mod endpoints;

use std::collections::HashMap;
use std::env;
use std::env::set_current_dir;
use std::error::Error;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::LazyLock;
use std::time::Duration;
use openssl::ssl::Ssl;
use tokio::net::*;
use tokio::*;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::timeout;
use tokio_openssl::SslStream;
use crate::requests::Request::{Get, Head, Options, Post, Trace, Put, Delete, Patch};
use crate::requests::*;
use crate::util::*;
use crate::config::CONFIG;
#[cfg(feature = "cgi")]
use crate::cgi::handle_cgi;
#[cfg(feature = "cgi")]
use crate::cgi::CGIStatus;
use crate::endpoints::ENDPOINT_LIBRARY;
use crate::error::ServerError;
#[cfg(feature = "cgi")]
use crate::pages::bad_gateway::bad_gateway;
use crate::pages::internal_server_error::internal_server_error;
use crate::ssl::SSL;
use crate::util::ResourceType::Dynamic;

async fn handle_connection<T>(
    stream: &mut T,
    keep_alive: &mut bool,
    local_ip: &IpAddr,
    remote_ip: &IpAddr,
    remote_port: &u16,
    #[cfg(feature = "cgi")]
    https: bool) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    match receive_request(stream, keep_alive).await {
        Ok(request) => {
            #[cfg(feature = "cgi")]
            match request {
                Get {resource, params, query_string, headers} => {
                    match &CONFIG.cgi {
                        Some(cgi) if cgi.enabled && cgi.should_attempt_cgi(&String::from((&resource).trim_start_matches("/"))) => {
                            match handle_cgi(stream, &headers, &resource, "GET", query_string, None, local_ip, remote_ip, remote_port, https).await {
                                Ok(CGIStatus::Available) | Ok(CGIStatus::Denied) => return Ok(()),
                                Err(_) => return bad_gateway(stream).await,
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                    handle_get(stream, &headers, resource, &params, local_ip, remote_ip, remote_port).await
                },
                Head {resource, params, query_string, headers} => {
                    match &CONFIG.cgi {
                        Some(cgi) if cgi.enabled && cgi.should_attempt_cgi(&String::from((&resource).trim_start_matches("/"))) => {
                            match handle_cgi(stream, &headers, &resource, "HEAD", query_string, None, local_ip, remote_ip, remote_port, https).await {
                                Ok(CGIStatus::Available) | Ok(CGIStatus::Denied) => return Ok(()),
                                Err(_) => return bad_gateway(stream).await,
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                    handle_head(stream, &headers, resource, &params, local_ip, remote_ip, remote_port).await
                },
                Post {resource, params, query_string, headers, data, cgi_data} => {
                    match &CONFIG.cgi {
                        Some(cgi) if cgi.enabled && cgi.should_attempt_cgi(&String::from((&resource).trim_start_matches("/"))) => {
                            match handle_cgi(stream, &headers, &resource, "POST", query_string, cgi_data, local_ip, remote_ip, remote_port, https).await {
                                Ok(CGIStatus::Available) | Ok(CGIStatus::Denied) => return Ok(()),
                                Err(_) => return bad_gateway(stream).await,
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                    handle_post(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await
                },
                Put {resource, params, query_string, headers, data, cgi_data} => {
                    match &CONFIG.cgi {
                        Some(cgi) if cgi.enabled && cgi.should_attempt_cgi(&String::from((&resource).trim_start_matches("/"))) => {
                            match handle_cgi(stream, &headers, &resource, "PUT", query_string, cgi_data, local_ip, remote_ip, remote_port, https).await {
                                Ok(CGIStatus::Available) | Ok(CGIStatus::Denied) => return Ok(()),
                                Err(_) => return bad_gateway(stream).await,
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                    handle_put(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await
                },
                Delete {resource, params, query_string, headers, data, cgi_data} => {
                    match &CONFIG.cgi {
                        Some(cgi) if cgi.enabled && cgi.should_attempt_cgi(&String::from((&resource).trim_start_matches("/"))) => {
                            match handle_cgi(stream, &headers, &resource, "DELETE", query_string, cgi_data, local_ip, remote_ip, remote_port, https).await {
                                Ok(CGIStatus::Available) | Ok(CGIStatus::Denied) => return Ok(()),
                                Err(_) => return bad_gateway(stream).await,
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                    handle_delete(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await
                },
                Options {..} =>
                    handle_options(stream).await,
                Patch {resource, params, query_string, headers, data, cgi_data} => {
                    match &CONFIG.cgi {
                        Some(cgi) if cgi.enabled && cgi.should_attempt_cgi(&String::from((&resource).trim_start_matches("/"))) => {
                            match handle_cgi(stream, &headers, &resource, "PATCH", query_string, cgi_data, local_ip, remote_ip, remote_port, https).await {
                                Ok(CGIStatus::Available) | Ok(CGIStatus::Denied) => return Ok(()),
                                Err(_) => return bad_gateway(stream).await,
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                    handle_patch(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await
                },
                Trace(request) if CONFIG.enable_trace => {
                    let response_headers: HashMap<String, String> = HashMap::from([
                        (String::from("Content-Type"), String::from("message/http"))
                    ]);

                    send_response(stream, 200, Some(response_headers), Some(request), None, Some(Dynamic)).await
                },
                _ => {
                    let accept_header = HashMap::from([
                        (String::from("Accept"), format!("GET, HEAD, POST,{} OPTIONS{}",
                                                         if (&*ENDPOINT_LIBRARY).is_some() {" PUT, DELETE, PATCH,"} else {""},
                                                         if CONFIG.enable_trace {", TRACE"} else {""}))
                    ]);

                    send_response(stream, 405, Some(accept_header), None, None, None).await
                }
            }
            #[cfg(not(feature = "cgi"))]
            match request {
                Get {resource, params, headers} =>
                    handle_get(stream, &headers, resource, &params, local_ip, remote_ip, remote_port).await,
                Head {resource, params, headers} =>
                    handle_head(stream, &headers, resource, &params, local_ip, remote_ip, remote_port).await,
                Post {resource, params, headers, data} =>
                    handle_post(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await,
                Put {resource, params, headers, data} =>
                    handle_put(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await,
                Delete {resource, params, headers, data} =>
                    handle_delete(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await,
                Options {..} =>
                    handle_options(stream).await,
                Patch {resource, params, headers, data} =>
                    handle_patch(stream, &headers, resource, &data, &params, local_ip, remote_ip, remote_port).await,
                Trace(request) if CONFIG.enable_trace => {
                    let response_headers: HashMap<String, String> = HashMap::from([
                        (String::from("Content-Type"), String::from("message/http"))
                    ]);

                    send_response(stream, 200, Some(response_headers), Some(request), None, Some(Dynamic)).await
                },
                _ => {
                    let accept_header = HashMap::from([
                        (String::from("Accept"), format!("GET, HEAD, POST,{} OPTIONS{}",
                                                         if (&*ENDPOINT_LIBRARY).is_some() {" PUT, DELETE, PATCH,"} else {""},
                                                         if CONFIG.enable_trace {", TRACE"} else {""}))
                    ]);

                    send_response(stream, 405, Some(accept_header), None, None, None).await
                }
            }
        },
        Err(e) => {
            match e {
                ServerError::DecompressionError(..) | ServerError::UnsupportedEncoding => {
                    send_response(stream, 406, None, None, None, None).await?
                },
                ServerError::InvalidRequest | ServerError::MalformedPayload => {
                    send_response(stream, 400, None, None, None, None).await?
                },
                ServerError::UnsupportedMediaType => {
                    let response_headers: HashMap<String, String> = HashMap::from([
                        (String::from("Accept"), String::from("application/x-www-form-urlencoded, multipart/form-data, text/plain, application/octet-stream")),
                        (String::from("Vary"), String::from("Content-Type"))
                    ]);

                    send_response(stream, 415, Some(response_headers), None, None, None).await?
                },
                ServerError::BodyTooLarge => {
                    send_response(stream, 413, None, None, None, None).await?
                },
                ServerError::VersionNotSupported => {
                    send_response(stream, 505, None, None, None, None).await?
                },
                _ => {
                    internal_server_error(stream).await?;
                }
            }
            Err(Box::new(e))
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    #[cfg(not(feature = "cgi"))]
    println!("Drain {}, starting...", env!("CARGO_PKG_VERSION"));
    #[cfg(feature = "cgi")]
    println!("Drain {} (CGI version), starting...", env!("CARGO_PKG_VERSION"));

    let bind_host = &CONFIG.bind_host;

    #[cfg(feature = "cgi")]
    match &CONFIG.cgi {
        Some(cgi) if cgi.enabled => {
            println!("CGI enabled. Scripts will be executed using {}", cgi.cgi_server);
        },
        _ => {
            println!("CGI disabled.");
        }
    }

    if CONFIG.be_verbose {
        match &CONFIG.encoding {
            Some(encoding) => {
                println!("Encoding enabled and set to \"{}\".", encoding.use_encoding);
            },
            _ => {
                println!("Encoding disabled.");
            }
        }

        println!("TRACE HTTP method is {}.\n\
                  Server header {} be sent.",
                if CONFIG.enable_trace { "enabled" } else { "disabled" },
                if CONFIG.enable_server_header { "will" } else { "won't" });

        println!("Request timeout will occur after {} seconds of inactivity from the client.", &CONFIG.request_timeout);
    }

    LazyLock::force(&ENDPOINT_LIBRARY);
    LazyLock::force(&SSL);

    #[cfg(target_family = "unix")]
    if *&*CHROOT {
        if let Err(e) = set_current_dir("/") {
            eprintln!("[main():{}] An error occurred while setting the current working directory.\n\
                                   Cannot continue any further, as it poses a threat to the data security.\n\
                                   Error information:\n", line!());
            return Err(e);
        }
    }

    match &*SSL {
        Some(ssl_info) => {
            let bind_port = ssl_info.port;
            let listener = TcpListener::bind(format!("{}:{}", bind_host, bind_port)).await?;
            println!("Listening on {}:{}", bind_host, bind_port);
            loop {
                let ssl = match Ssl::new(&ssl_info.ctx) {
                    Ok(ssl) => ssl,
                    Err(e) => {
                        eprintln!("[main():{}] An error occurred while establishing a secure connection.\n\
                                               Error information:\n{e}\n\
                                               Continuing with the regular HTTP...", line!());

                        break;
                    }
                };

                let (stream, _) = listener.accept().await?;
                let local_addr = match stream.local_addr() {
                    Ok(addr) => addr,
                    Err(e) => {
                        eprintln!("[main():{}] An error occurred while getting the server's address.\n\
                                               Error information:\n{e}", line!());

                        continue;
                    }
                };
                let remote_addr = match stream.peer_addr() {
                    Ok(addr) => addr,
                    Err(e) => {
                        eprintln!("[main():{}] An error occurred while getting the client's address.\n\
                                               Error information:\n{e}", line!());

                        continue;
                    }
                };

                let local_ip = local_addr.ip();
                let remote_ip = remote_addr.ip();
                let remote_port = remote_addr.port();

                let mut stream = SslStream::new(ssl, stream)?;
                if let Err(e) = Pin::new(&mut stream).accept().await {
                    if let Some(ssl_error) = e.ssl_error() {
                        if ssl_error.to_string().contains("http request") {
                            continue;
                        }
                    }

                    eprintln!("[main():{}] An error occurred while establishing a secure connection.\n\
                                           Error information:\n{e}\n\
                                           Continuing with the regular HTTP...", line!());

                    break;
                }

                spawn(async move {
                    let mut keep_alive = true;
                    let mut buf: [u8; 1] = [0; 1];
                    loop {
                        if !keep_alive {
                            break;
                        }

                        match timeout(Duration::from_secs((&CONFIG).request_timeout), Pin::new(&mut stream).peek(&mut buf)).await {
                            Ok(Ok(0)) | Err(_) => break,
                            Ok(Err(e)) => {
                                if e.to_string().eq("the SSL session has been shut down") {
                                    break;
                                }

                                eprintln!("[main():{}] An error occurred while handling connection:\n{e}\n", line!());
                                break;
                            },
                            _ => {}
                        }

                        #[cfg(feature = "cgi")]
                        let https_enabled = true;

                        if let Err(e) = handle_connection(
                            &mut stream,
                            &mut keep_alive,
                            &local_ip,
                            &remote_ip,
                            &remote_port,
                            #[cfg(feature = "cgi")]
                            https_enabled
                        ).await {
                            eprintln!("[main():{}] An error occurred while handling connection:\
                            \n{e}\n", line!());
                        }
                    }
                });
            }
        },
        _ => {}
    }

    let bind_port = &CONFIG.bind_port;
    let listener = TcpListener::bind(format!("{}:{}", bind_host, bind_port)).await?;
    println!("Listening on {}:{}", bind_host, bind_port);
    loop {
        let (mut stream, _) = listener.accept().await?;
        let local_addr = match stream.local_addr() {
            Ok(addr) => addr,
            Err(e) => {
                eprintln!("[main():{}] An error occurred while getting the server's address.\n\
                                       Error information:\n{e}", line!());

                continue;
            }
        };
        let remote_addr = match stream.peer_addr() {
            Ok(addr) => addr,
            Err(e) => {
                eprintln!("[main():{}] An error occurred while getting the client's address.\n\
                                       Error information:\n{e}", line!());

                continue;
            }
        };

        let local_ip = local_addr.ip();
        let remote_ip = remote_addr.ip();
        let remote_port = remote_addr.port();

        spawn(async move {
            let mut keep_alive = true;
            let mut buf: [u8; 1] = [0; 1];
            loop {
                if !keep_alive {
                    break;
                }

                match timeout(Duration::from_secs((&CONFIG).request_timeout), stream.peek(&mut buf)).await {
                    Ok(Ok(0)) | Err(_) => break,
                    Ok(Err(e)) => {
                        eprintln!("[main():{}] An error occurred while handling connection:\n{e}\n", line!());
                        break;
                    },
                    _ => {}
                }

                #[cfg(feature = "cgi")]
                let https_enabled = false;

                if let Err(e) = handle_connection(
                    &mut stream,
                    &mut keep_alive,
                    &local_ip,
                    &remote_ip,
                    &remote_port,
                    #[cfg(feature = "cgi")]
                    https_enabled
                ).await {
                    eprintln!("[main():{}] An error occurred while handling connection:\n{e}\n", line!());
                }
            }
        });
    }
}