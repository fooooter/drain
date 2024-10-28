use std::collections::HashMap;
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
    pub bind: String
}

pub async fn get_config(mut stream: Option<&mut TcpStream>) -> Config {
    let mut json_str: String = String::new();
    let config_file = File::open("config.json").await;

    if let Some(s) = stream {
        match config_file {
            Ok(mut f) => {
                rts_wrapper(&mut f, &mut json_str, s).await;
            },
            Err(e1) => {
                eprintln!("[get_config():26] A critical server config file wasn't found.\n\
                           Error information:\n\
                           {:?}\n\
                           Attempting to send Internal Server Error page to the client...", e1);
                if let Err(e2) = internal_server_error(s).await {
                    eprintln!("[get_config():31] FAILED. Error information: {:?}", e2);
                }
                eprintln!("Attempting to close connection...");
                if let Err(e2) = s.shutdown().await {
                    eprintln!("[get_config():35] FAILED. Error information:\n{:?}", e2);
                }
                panic!("Unrecoverable error occurred while handling connection.");
            }
        }

        match serde_json::from_str(&*json_str) {
            Ok(json) => json,
            Err(e1) => {
                eprintln!("[get_config():44] A critical server config file is malformed.\n\
                               Error information:\n\
                               {:?}\n\
                               Attempting to send Internal Server Error page to the client...", e1);
                if let Err(e2) = internal_server_error(s).await {
                    eprintln!("[get_config():49] FAILED. Error information: {:?}", e2);
                }
                eprintln!("Attempting to close connection...");
                if let Err(e2) = s.shutdown().await {
                    eprintln!("[get_config():53] FAILED. Error information:\n{:?}", e2);
                }
                panic!("Unrecoverable error occurred while handling connection.");
            }
        }
    } else {
        match config_file {
            Ok(mut f) => {
                if let Err(e) = f.read_to_string(&mut json_str).await {
                    eprintln!("[get_config():62] An error occurred after an attempt to read from a file: {:?}.\n\
                               Error information:\n\
                               {:?}\n", f, e);
                    panic!("Unrecoverable error occurred while trying to set up connection.");
                }
            },
            Err(e1) => {
                eprintln!("[get_config():69] A critical server config file wasn't found.\n\
                            Error information:\n\
                            {:?}", e1);
                panic!("Unrecoverable error occurred trying to set up connection.");
            }
        }

        match serde_json::from_str(&*json_str) {
            Ok(json) => json,
            Err(e) => {
                eprintln!("[get_config():79] A critical server config file is malformed.\n\
                           Error information:\n\
                           {:?}", e);
                panic!("Unrecoverable error occurred trying to set up connection.");
            }
        }
    }
}

pub async fn config(stream: Option<&mut TcpStream>) -> Config {
    Box::pin(get_config(stream)).await
}