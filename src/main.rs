mod requests;
mod util;
mod pages;
mod config;
mod error;
use std::collections::HashMap;
use std::error::Error;
use std::pin::Pin;
use openssl::error::ErrorStack;
use openssl::ssl::{select_next_proto, AlpnError, Ssl, SslContext, SslFiletype, SslMethod, SslVerifyMode, SslVersion};
use tokio::net::*;
use tokio::*;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslStream;
use crate::requests::Request::{Get, Head, Options, Post};
use crate::requests::*;
use crate::util::*;
use crate::config::CONFIG;
use crate::error::ServerError;
use crate::pages::internal_server_error::internal_server_error;

fn configure_ssl() -> Result<Ssl, ErrorStack> {
    let server_root = &CONFIG.server_root;
    let mut ssl_ctx_builder = SslContext::builder(SslMethod::tls())?;

    ssl_ctx_builder.set_private_key_file(format!("{}/{}", server_root, &CONFIG.https.ssl_private_key_file), SslFiletype::PEM)?;
    ssl_ctx_builder.set_certificate_file(format!("{}/{}", server_root, &CONFIG.https.ssl_certificate_file), SslFiletype::PEM)?;
    ssl_ctx_builder.check_private_key()?;

    ssl_ctx_builder.set_min_proto_version(
        match &CONFIG.https.min_protocol_version {
            Some(min_proto_version) if min_proto_version.eq("SSL3") => Some(SslVersion::SSL3),
            Some(min_proto_version) if min_proto_version.eq("TLS1.3") => Some(SslVersion::TLS1_3),
            Some(min_proto_version) if min_proto_version.eq("TLS1") => Some(SslVersion::TLS1),
            Some(min_proto_version) if min_proto_version.eq("DTLS1") => Some(SslVersion::DTLS1),
            Some(min_proto_version) if min_proto_version.eq("DTLS1.2") => Some(SslVersion::DTLS1_2),
            Some(min_proto_version) if min_proto_version.eq("TLS1.1") => Some(SslVersion::TLS1_1),
            Some(min_proto_version) if min_proto_version.eq("TLS1.2") => Some(SslVersion::TLS1_2),
            Some(_) | None => None
        }
    )?;

    if let Some(SslVersion::TLS1_3) = ssl_ctx_builder.min_proto_version() {
        ssl_ctx_builder.set_ciphersuites(&CONFIG.https.cipher_list)?;
    } else {
        ssl_ctx_builder.set_cipher_list(&CONFIG.https.cipher_list)?;
    }

    ssl_ctx_builder.set_verify(SslVerifyMode::PEER);
    ssl_ctx_builder.set_alpn_select_callback(|_ssl, client_protocols| {
        if let Some(p) = select_next_proto(b"\x08http/1.1", client_protocols) {
            Ok(p)
        } else {
            Err(AlpnError::ALERT_FATAL)
        }
    });

    let ssl_ctx = ssl_ctx_builder.build();
    Ssl::new(&ssl_ctx)
}

async fn handle_connection<T>(mut stream: T) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    match receive_request(&mut stream).await {
        Ok(request) => {
            match request {
                Get {resource, params, headers} => handle_get(stream, &headers, resource, &params).await,
                Head {resource, headers} => handle_head(stream, &headers, resource).await,
                Post {resource, headers, data} => handle_post(stream, &headers, resource, &data).await,
                Options {..} => handle_options(stream).await,
                _ => {
                    let accept_header = HashMap::from([(String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))]);

                    send_response(&mut stream, 405, Some(accept_header), None, None).await
                }
            }
        },
        Err(e) => {
            match e {
                ServerError::DecompressionError(..) | ServerError::UnsupportedEncoding => {
                    send_response(&mut stream, 406, None, None, None).await?
                },
                ServerError::InvalidRequest | ServerError::MalformedPayload => {
                    send_response(&mut stream, 400, None, None, None).await?
                },
                ServerError::UnsupportedMediaType => {
                    let response_headers: HashMap<String, String> = HashMap::from([
                        (String::from("Accept-Post"), String::from("application/x-www-form-urlencoded, multipart/form-data")),
                        (String::from("Vary"), String::from("Content-Type"))
                    ]);

                    send_response(&mut stream, 415, Some(response_headers), None, None).await?
                }
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

    let https_enabled = (&CONFIG.https.enabled).clone();
    let bind_host = &CONFIG.bind_host;

    if let Some(encoding) = &CONFIG.encoding {
        println!("Encoding enabled and set to \"{}\".", encoding.use_encoding);
    } else {
        println!("Encoding disabled.");
    }

    if https_enabled {
        println!("SSL enabled.");
        let bind_port = &CONFIG.https.bind_port;
        let listener = TcpListener::bind(format!("{}:{}", bind_host, bind_port)).await?;
        println!("Listening on {}:{}", bind_host, bind_port);
        loop {
            let ssl = configure_ssl();
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