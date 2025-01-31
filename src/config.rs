use std::collections::HashMap;
use std::env;
use std::sync::LazyLock;
use glob::glob;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::runtime::Handle;
use tokio::task;

#[derive(Deserialize)]
pub struct AccessControl {
    pub deny_action: u16,
    pub list: HashMap<String, String>
}

#[derive(Deserialize)]
pub struct Encoding {
    pub use_encoding: String,
    pub supported_encodings: Vec<String>,
    pub encoding_applicable_mime_types: Option<Vec<String>>
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
    max_content_length: Option<usize>,
    pub global_response_headers: Option<HashMap<String, String>>,
    pub access_control: Option<AccessControl>,
    pub bind_host: String,
    pub bind_port: String,
    pub endpoints: Option<Vec<String>>,
    pub endpoints_library: String,
    pub encoding: Option<Encoding>,
    pub document_root: String,
    pub server_root: String,
    pub https: Https
}

impl Config {
    pub async fn new() -> Self {
        let config_path = env::var("DRAIN_CONFIG");
        let config_file;

        match &config_path {
            Ok(c_f) => {
                config_file = File::open(c_f).await;
                println!("Config path: {c_f}");
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

        let config: Config = match serde_json::from_slice(&*json) {
            Ok(json) => json,
            Err(e) => {
                eprintln!("[Config::new():{}] A critical server config file is malformed.\n\
                           Error information:\n\
                           {e}", line!());
                panic!("Unrecoverable error occurred while trying to set up connection.");
            }
        };

        if let Some(access_control) = &config.access_control {
            if access_control.deny_action != 404 && access_control.deny_action != 403 {
                eprintln!("[Config::new():{}]   A critical server config file is malformed.\n\
                                                Error information:\n\
                                                invalid deny action in config.json access_control, should be either 404 or 403", line!());
                panic!("Unrecoverable error occurred while trying to set up connection.")
            }

            for (_, v) in &access_control.list {
                if !v.eq("allow") && !v.eq("deny") {
                    eprintln!("[Config::new():{}]   A critical server config file is malformed.\n\
                                                    Error information:\n\
                                                    invalid word in config.json access_control, should be either \"allow\" or \"deny\"", line!());

                    panic!("Unrecoverable error occurred while trying to set up connection.");
                }
            }
        }

        if let Some(encoding) = &config.encoding {
            if !encoding.supported_encodings.contains(&encoding.use_encoding) {
                eprintln!("[Config::new():{}]   A critical server config file is malformed.\n\
                                                Error information:\n\
                                                invalid word in config.json use_encoding, should be either \"gzip\" or \"br\"\n\
                                                if you specified either \"gzip\" or \"br\" and still got this error, make sure it's specified in supported_encodings", line!());

                panic!("Unrecoverable error occurred while trying to set up connection.");
            }
        }

        config
    }

    pub fn get_max_content_length(&self) -> usize {
        if let Some(max_content_length) = self.max_content_length {
            return max_content_length;
        }
        1073741824
    }

    pub fn get_supported_encodings(&self) -> Option<&Vec<String>> {
        if let Some(encoding) = &self.encoding {
            let supported_encodings = &encoding.supported_encodings;

            if !supported_encodings.is_empty() {
                return Some(supported_encodings);
            }
        }
        None
    }

    pub fn get_response_encoding(&self, content: &Vec<u8>, type_guess: &String, type_: &String, headers: &HashMap<String, String>) -> Option<&String> {
        if let Some(encoding) = &self.encoding {
            if let Some(content_encoding) = headers.get("accept-encoding") {
                let content_empty = content.is_empty();
                let type_equals_text = type_.eq("text");
                if !content_empty && type_equals_text {
                    let encoding = &encoding.use_encoding;
                    let accepted_encodings: Vec<String> = content_encoding.split(',').map(|x| String::from(x.trim())).collect();

                    if accepted_encodings.contains(&encoding) {
                        return Some(encoding);
                    }
                    return None;
                }
                if !content_empty && !type_equals_text {
                    if let Some(encoding_applicable_mime_types) = &encoding.encoding_applicable_mime_types {
                        if encoding_applicable_mime_types.contains(type_guess) {
                            let encoding = &encoding.use_encoding;
                            let accepted_encodings: Vec<String> = content_encoding.split(',').map(|x| String::from(x.trim())).collect();

                            if accepted_encodings.contains(&encoding) {
                                return Some(encoding);
                            }
                        }
                    }
                }
            }
        }
        None
    }


}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    task::block_in_place(move || {
        Handle::current().block_on(async move {
            Config::new().await
        })
    })
});

impl AccessControl {
    pub fn is_access_allowed(&self, resource: &String) -> bool {
        for (k, v) in &self.list {
            if let Ok(paths) = glob(&*format!("{}/{k}", &CONFIG.document_root)) {
                for entry in paths.filter_map(Result::ok) {
                    if entry.to_string_lossy().eq(&*format!("{}/{resource}", &CONFIG.document_root)) {
                        if v.eq("deny") {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}