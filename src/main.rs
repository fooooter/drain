mod requests;
mod util;
mod pages;
mod config;
mod error;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::pin::Pin;
use std::sync::LazyLock;
use tokio::net::*;
use tokio::*;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslStream;
use crate::requests::Request::{Get, Head, Options, Post, Trace};
use crate::requests::*;
use crate::util::*;
use crate::config::CONFIG;
use crate::error::ServerError;
use crate::pages::internal_server_error::internal_server_error;
use crate::util::ResourceType::Dynamic;

async fn handle_connection<T>(mut stream: T) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    match receive_request(&mut stream).await {
        Ok(request) => {
            match request {
                Get {resource, params, headers} => handle_get(stream, &headers, resource, &params).await,
                Head {resource, params, headers} => handle_head(stream, &headers, resource, &params).await,
                Post {resource, params, headers, data} => handle_post(stream, &headers, resource, &data, &params).await,
                Options {..} => handle_options(stream).await,
                Trace(request) if CONFIG.enable_trace => {
                    let response_headers: HashMap<String, String> = HashMap::from([
                        (String::from("Content-Type"), String::from("message/http"))
                    ]);

                    send_response(&mut stream, 200, Some(response_headers), Some(request), None, Some(Dynamic)).await
                }
                _ => {
                    let accept_header = HashMap::from([
                        (String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))
                    ]);

                    send_response(&mut stream, 405, Some(accept_header), None, None, None).await
                }
            }
        },
        Err(e) => {
            match e {
                ServerError::DecompressionError(..) | ServerError::UnsupportedEncoding => {
                    send_response(&mut stream, 406, None, None, None, None).await?
                },
                ServerError::InvalidRequest | ServerError::MalformedPayload => {
                    send_response(&mut stream, 400, None, None, None, None).await?
                },
                ServerError::UnsupportedMediaType => {
                    let response_headers: HashMap<String, String> = HashMap::from([
                        (String::from("Accept-Post"), String::from("application/x-www-form-urlencoded, multipart/form-data")),
                        (String::from("Vary"), String::from("Content-Type"))
                    ]);

                    send_response(&mut stream, 415, Some(response_headers), None, None, None).await?
                },
                ServerError::BodyTooLarge => {
                    send_response(&mut stream, 413, None, None, None, None).await?
                },
                _ => {
                    internal_server_error(&mut stream).await?;
                }
            }
            Err(Box::new(e))
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    println!("Drain {}, starting...", env!("CARGO_PKG_VERSION"));

    let bind_host = &CONFIG.bind_host;

    if CONFIG.be_verbose {
        if let Some(encoding) = &CONFIG.encoding {
            println!("Encoding enabled and set to \"{}\".", encoding.use_encoding);
        } else {
            println!("Encoding disabled.");
        }

        println!("TRACE HTTP method is {}.\n\
                  Server header {} be sent.",
                if CONFIG.enable_trace { "enabled" } else { "disabled" },
                if CONFIG.enable_server_header { "will" } else { "won't" });
    }

    println!("Initializing the library...");

    LazyLock::force(&ENDPOINT_LIBRARY);

    match &CONFIG.https {
        Some(https) if https.enabled => {
            println!("SSL enabled.");
            let bind_port = &https.bind_port;
            let listener = TcpListener::bind(format!("{}:{}", bind_host, bind_port)).await?;
            println!("Listening on {}:{}", bind_host, bind_port);
            loop {
                let ssl = https.configure_ssl();
                match ssl {
                    Ok(ssl) => {
                        let (stream, _) = listener.accept().await?;
                        let mut stream = SslStream::new(ssl, stream)?;
                        if let Err(e1) = Pin::new(&mut stream).accept().await {
                            eprintln!("[main():{}] An error occurred while establishing a secure connection.\n\
                                        Error information:\n{e1}\n\
                                        Continuing with the regular HTTP...", line!());

                            break;
                        }

                        spawn(async move {
                            if let Err(e) = handle_connection(stream).await {
                                eprintln!("[main():{}] An error occurred while handling connection:\
                                \n{e}\n", line!());
                            }
                        });
                    },
                    Err(e) => {
                        eprintln!("[main():{}] An error occurred while configuring SSL for a secure connection.\n\
                                    Error information:\n{e}\n\
                                    Continuing with the regular HTTP.", line!());

                        break;
                    }
                }
            }
        },
        _ => {}
    }

    println!("SSL disabled.");
    let bind_port = &CONFIG.bind_port;
    let listener = TcpListener::bind(format!("{}:{}", bind_host, bind_port)).await?;
    println!("Listening on {}:{}", bind_host, bind_port);
    loop {
        let (stream, _) = listener.accept().await?;

        spawn(async move {
            if let Err(e) = handle_connection(stream).await {
                eprintln!("[main():{}] An error occurred while handling connection:\n{e}\n", line!());
            }
        });
    }
}