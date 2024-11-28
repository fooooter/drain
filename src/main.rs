mod requests;
mod util;
mod pages;
mod config;
mod error;
use std::collections::HashMap;
use std::error::Error;
use std::env::set_current_dir;
use std::pin::Pin;
use openssl::error::ErrorStack;
use openssl::ssl::{Ssl, SslContext, SslFiletype, SslMethod, SslVerifyMode, SslVersion};
use tokio::net::*;
use tokio::*;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslStream;
use crate::requests::Request::{Get, Head, Options, Post};
use crate::requests::*;
use crate::util::*;
use crate::config::Config;
use crate::error::ServerError;

fn configure_ssl(config: &Config) -> Result<Ssl, ErrorStack> {
    let mut ssl_ctx_builder = SslContext::builder(SslMethod::tls())?;

    ssl_ctx_builder.set_private_key_file(&config.https.ssl_private_key_file, SslFiletype::PEM)?;
    ssl_ctx_builder.set_certificate_file(&config.https.ssl_certificate_file, SslFiletype::PEM)?;
    ssl_ctx_builder.check_private_key()?;

    ssl_ctx_builder.set_min_proto_version(Some(SslVersion::TLS1_3))?;
    ssl_ctx_builder.set_alpn_protos(b"\x08http/1.1")?;
    ssl_ctx_builder.set_ciphersuites("TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256")?;
    ssl_ctx_builder.set_verify(SslVerifyMode::NONE);

    let ssl_ctx = ssl_ctx_builder.build();
    Ssl::new(&ssl_ctx)
}

async fn handle_connection<T>(mut stream: T) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let config = Config::new(Some(&mut stream)).await;
    let document_root = &config.document_root;
    set_current_dir(document_root)?;

    match receive_request(&mut stream, &config).await {
        Ok(request) => {
            match request {
                Get {resource, params, headers} => handle_get(stream, config, &headers, resource, &params).await,
                Head {resource, headers} => handle_head(stream, config, &headers, resource).await,
                Post {resource, headers, data} => handle_post(stream, config, &headers, resource, &data).await,
                Options {resource, headers} => handle_options(stream, config, &headers, resource).await,
                _ => {
                    let accept_header = HashMap::from([(String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))]);

                    send_response(&mut stream, Some(config), 405, Some(accept_header), None).await
                }
            }
        },
        Err(e) => {
            if let ServerError::DecompressionError(..) = e {
                send_response(&mut stream, Some(config), 406, None, None).await?;
            } else {
                send_response(&mut stream, Some(config), 400, None, None).await?;
            }
            Err(Box::new(e))
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let config = Config::new::<TcpStream>(None).await;
    let https_enabled = (&config.https.enabled).clone();

    let listener_http = TcpListener::bind(&config.bind).await?;

    if https_enabled {
        let listener_https = TcpListener::bind(&config.https.bind).await?;

        loop {
            set_current_dir(&config.server_root)?;
            let ssl = configure_ssl(&config);
            match ssl {
                Ok(ssl) => {
                    let (stream, _) = listener_https.accept().await?;
                    let mut stream = SslStream::new(ssl, stream)?;
                    Pin::new(&mut stream).accept().await.unwrap();

                    spawn(async move {
                        if let Err(e) = handle_connection(stream).await {
                            eprintln!("[main():{}] An error occurred while handling connection:\n{e}\n", line!());
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

    loop {
        let (stream, _) = listener_http.accept().await?;

        spawn(async move {
            if let Err(e) = handle_connection(stream).await {
                eprintln!("[main():{}] An error occurred while handling connection:\n{e}\n", line!());
            }
        });
    }
}