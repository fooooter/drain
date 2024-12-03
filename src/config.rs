use std::collections::HashMap;
use std::env;
use glob::glob;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use crate::pages::internal_server_error::internal_server_error;

#[derive(Deserialize)]
pub struct AccessControl {
    pub deny_action: u16,
    pub list: HashMap<String, String>
}

#[derive(Deserialize)]
pub struct Encoding {
    pub enabled: bool,
    pub supported_encodings: Vec<String>,
    pub use_encoding: String,
    pub encoding_applicable_mime_types: Vec<String>
}

#[derive(Deserialize)]
pub struct Https {
    pub enabled: bool,
    pub bind_port: String,
    pub min_protocol_version: Option<String>,
    pub cipher_list: String,
    pub ssl_private_key_file: String,
    pub ssl_certificate_file: String
}

#[derive(Deserialize)]
pub struct Config {
    pub global_response_headers: HashMap<String, String>,
    pub access_control: AccessControl,
    pub bind_host: String,
    pub bind_port: String,
    pub dynamic_pages: Vec<String>,
    pub dynamic_pages_library: String,
    pub encoding: Encoding,
    pub document_root: String,
    pub server_root: String,
    pub https: Https
}

impl Config {
    pub async fn new() -> Self {
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

        let mut json: Vec<u8> = Vec::new();
        match config_file {
            Ok(mut f) => {
                if let Err(e) = f.read_to_end(&mut json).await {
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

        match serde_json::from_slice(&*json) {
            Ok(json) => json,
            Err(e) => {
                eprintln!("[Config::new():{}] A critical server config file is malformed.\n\
                           Error information:\n\
                           {e}", line!());
                panic!("Unrecoverable error occurred while trying to set up connection.");
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
        let supported_encodings = &self.encoding.supported_encodings;

        if supported_encodings.is_empty() {
            return None
        }
        Some(supported_encodings)
    }

    pub fn get_response_encoding(&self, content: &Vec<u8>, type_guess: &String, type_: &String, headers: &HashMap<String, String>) -> Option<&String> {
        if let (true, false, true, Some(content_encoding), Some(supported_encodings)) = (
                                                      &self.encoding.enabled,
                                                      content.is_empty(),
                                                      type_.eq("text") ||
                                                      self.encoding.encoding_applicable_mime_types.contains(type_guess),
                                                      headers.get("accept-encoding"),
                                                      &self.get_supported_encodings())
        {
            let encoding = &self.encoding.use_encoding;
            let accepted_encodings: Vec<String> = content_encoding.split(',').map(|x| String::from(x.trim())).collect();

            if accepted_encodings.contains(&encoding) && supported_encodings.contains(&encoding) {
                return Some(encoding)
            }
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