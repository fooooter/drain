use std::collections::HashMap;
use std::env;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use crate::pages::internal_server_error::internal_server_error;
use crate::util::rts_wrapper;

#[derive(Deserialize)]
pub struct Config {
    pub global_response_headers: HashMap<String, String>,
    pub access_control: HashMap<String, String>,
    pub bind: String,
    pub dynamic_pages: Vec<String>,
    pub dynamic_pages_library: String,
    pub supported_encodings: Vec<String>,
    pub use_encoding: String,
    pub document_root: String
}

pub async fn get_config(stream: Option<&mut TcpStream>) -> Config {
    let config_path = env::var("WEB_SERVER_CONFIG");
    let config_file;

    match &config_path {
        Ok(c_f) => {
            config_file = File::open(c_f).await;
        }
        Err(e) => {
            eprintln!("[get_config():{}] A critical server config file wasn't found.\n\
                        Error information:\n\
                        {}", line!(), *e);
            panic!("Unrecoverable error occurred while trying to set up connection.");
        }
    }

    let mut json_str: String = String::new();

    if let Some(s) = stream {
        match config_file {
            Ok(mut f) => {
                rts_wrapper(&mut f, &mut json_str, s).await;
            },
            Err(e1) => {
                eprintln!("[get_config():{}] A critical server config file wasn't found.\n\
                           Error information:\n\
                           {e1}\n\
                           Attempting to send Internal Server Error page to the client...", line!());
                if let Err(e2) = internal_server_error(s).await {
                    eprintln!("[get_config():{}] FAILED. Error information: {e2}", line!());
                }
                eprintln!("Attempting to close connection...");
                if let Err(e2) = s.shutdown().await {
                    eprintln!("[get_config():{}] FAILED. Error information:\n{e2}", line!());
                }
                panic!("Unrecoverable error occurred while handling connection.");
            }
        }

        match serde_json::from_str(&*json_str) {
            Ok(json) => json,
            Err(e1) => {
                eprintln!("[get_config():{}] A critical server config file is malformed.\n\
                               Error information:\n\
                               {e1}\n\
                               Attempting to send Internal Server Error page to the client...", line!());
                if let Err(e2) = internal_server_error(s).await {
                    eprintln!("[get_config():{}] FAILED. Error information: {e2}", line!());
                }
                eprintln!("Attempting to close connection...");
                if let Err(e2) = s.shutdown().await {
                    eprintln!("[get_config():{}] FAILED. Error information:\n{e2}", line!());
                }
                panic!("Unrecoverable error occurred while handling connection.");
            }
        }
    } else {
        match config_file {
            Ok(mut f) => {
                if let Err(e) = f.read_to_string(&mut json_str).await {
                    eprintln!("[get_config():{}] An error occurred after an attempt to read from a file: {:?}.\n\
                               Error information:\n\
                               {e}\n", line!(), f);
                    panic!("Unrecoverable error occurred while trying to set up connection.");
                }
            },
            Err(e) => {
                eprintln!("[get_config():{}] A critical server config file wasn't found.\n\
                            Error information:\n\
                            {e}", line!());
                panic!("Unrecoverable error occurred while trying to set up connection.");
            }
        }

        match serde_json::from_str(&*json_str) {
            Ok(json) => json,
            Err(e) => {
                eprintln!("[get_config():{}] A critical server config file is malformed.\n\
                           Error information:\n\
                           {e}", line!());
                panic!("Unrecoverable error occurred while trying to set up connection.");
            }
        }
    }
}

pub async fn config(stream: Option<&mut TcpStream>) -> Config {
    Box::pin(get_config(stream)).await
}