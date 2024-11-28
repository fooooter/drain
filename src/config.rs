use std::collections::HashMap;
use std::env;
use glob::glob;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use crate::pages::internal_server_error::internal_server_error;
use crate::util::rts_wrapper;

#[derive(Deserialize)]
pub struct AccessControl {
    pub deny_action: u16,
    pub list: HashMap<String, String>
}

#[derive(Deserialize)]
pub struct Https {
    pub enabled: bool,
    pub bind: String,
    pub ssl_private_key_file: String,
    pub ssl_certificate_file: String
}

#[derive(Deserialize)]
pub struct Config {
    pub global_response_headers: HashMap<String, String>,
    pub access_control: AccessControl,
    pub bind: String,
    pub dynamic_pages: Vec<String>,
    pub dynamic_pages_library: String,
    pub supported_encodings: Vec<String>,
    pub use_encoding: String,
    pub document_root: String,
    pub server_root: String,
    pub https: Https
}

impl Config {
    pub async fn new<T>(stream: Option<&mut T>) -> Self
    where
        T: AsyncRead + AsyncWrite + Unpin
    {
        let config_path = env::var("WEB_SERVER_CONFIG");
        let config_file;

        match &config_path {
            Ok(c_f) => {
                config_file = File::open(c_f).await;
            }
            Err(e) => {
                eprintln!("[Config::new():{}] A critical server config file wasn't found.\n\
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
                    eprintln!("[Config::new():{}] A critical server config file wasn't found.\n\
                               Error information:\n\
                               {e1}\n\
                               Attempting to send Internal Server Error page to the client...", line!());
                    if let Err(e2) = internal_server_error(s).await {
                        eprintln!("[Config::new():{}] FAILED. Error information: {e2}", line!());
                    }
                    eprintln!("Attempting to close connection...");
                    if let Err(e2) = s.shutdown().await {
                        eprintln!("[Config::new():{}] FAILED. Error information:\n{e2}", line!());
                    }
                    panic!("Unrecoverable error occurred while handling connection.");
                }
            }

            match serde_json::from_str(&*json_str) {
                Ok(json) => json,
                Err(e1) => {
                    eprintln!("[Config::new():{}] A critical server config file is malformed.\n\
                                   Error information:\n\
                                   {e1}\n\
                                   Attempting to send Internal Server Error page to the client...", line!());
                    if let Err(e2) = internal_server_error(s).await {
                        eprintln!("[Config::new():{}] FAILED. Error information: {e2}", line!());
                    }
                    eprintln!("Attempting to close connection...");
                    if let Err(e2) = s.shutdown().await {
                        eprintln!("[Config::new():{}] FAILED. Error information:\n{e2}", line!());
                    }
                    panic!("Unrecoverable error occurred while handling connection.");
                }
            }
        } else {
            match config_file {
                Ok(mut f) => {
                    if let Err(e) = f.read_to_string(&mut json_str).await {
                        eprintln!("[Config::new():{}] An error occurred after an attempt to read from a file: {:?}.\n\
                                   Error information:\n\
                                   {e}\n", line!(), f);
                        panic!("Unrecoverable error occurred while trying to set up connection.");
                    }
                },
                Err(e) => {
                    eprintln!("[Config::new():{}] A critical server config file wasn't found.\n\
                                Error information:\n\
                                {e}", line!());
                    panic!("Unrecoverable error occurred while trying to set up connection.");
                }
            }

            match serde_json::from_str(&*json_str) {
                Ok(json) => json,
                Err(e) => {
                    eprintln!("[Config::new():{}] A critical server config file is malformed.\n\
                               Error information:\n\
                               {e}", line!());
                    panic!("Unrecoverable error occurred while trying to set up connection.");
                }
            }
        }
    }

    pub async fn is_access_allowed<T>(&self, resource: &String, stream: &mut T) -> bool
    where
        T: AsyncRead + AsyncWrite + Unpin
    {
        for (k, v) in &self.access_control.list {
            if let Ok(paths) = glob(&*k) {
                for entry in paths.filter_map(Result::ok) {
                    if entry.to_string_lossy().eq(resource) {
                        if v.eq("deny") {
                            return false;
                        }

                        if !v.eq("allow") {
                            eprintln!("[is_access_allowed():{}] A critical server config file is malformed.\n\
                                        Error information:\n\
                                        invalid word in config.json access_control, should be either \"allow\" or \"deny\"\n\
                                        Attempting to send Internal Server Error page to the client...", line!());

                            if let Err(e) = internal_server_error(stream).await {
                                eprintln!("[is_access_allowed():{}] FAILED. Error information: {e}", line!());
                            }
                            eprintln!("Attempting to close connection...");
                            if let Err(e) = stream.shutdown().await {
                                eprintln!("[is_access_allowed():{}] FAILED. Error information:\n{e}", line!());
                            }
                            panic!("Unrecoverable error occurred trying to set up connection.");
                        }

                        return true;
                    }
                }
            }
        }
        true
    }

    pub fn get_supported_encodings(&self) -> Option<&Vec<String>> {
        let supported_encodings = &self.supported_encodings;

        if supported_encodings.is_empty() {
            return None
        }
        Some(supported_encodings)
    }

    pub fn get_response_encoding(&self, headers: &HashMap<String, String>) -> Option<&String> {
        let encoding = &self.use_encoding;

        if let (Some(content_encoding), Some(supported_encodings)) = (headers.get("Accept-Encoding"), &self.get_supported_encodings()) {
            let accepted_encodings: Vec<String> = content_encoding.split(',').map(|x| String::from(x.trim())).collect();

            if accepted_encodings.contains(&encoding) && supported_encodings.contains(&encoding) {
                return Some(encoding)
            }
            return None
        }
        None
    }

    pub fn get_deny_action(&self) -> u16 {
        let deny_action = (&self.access_control.deny_action).clone();
        if deny_action != 403 && deny_action != 404 {
            return 404
        }
        deny_action
    }
}