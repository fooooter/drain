use std::collections::HashMap;
use std::env;
use std::sync::LazyLock;
use glob::glob;
use openssl::error::ErrorStack;
use openssl::ssl::{select_next_proto, AlpnError, SslContext, SslFiletype, SslMethod, SslOptions, SslSessionCacheMode, SslVerifyMode, SslVersion};
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::runtime::Handle;
use tokio::task;
#[cfg(target_family = "unix")]
use crate::util::CHROOT;

#[derive(Deserialize)]
pub struct AccessControl {
    pub deny_action: u16,
    list: HashMap<String, String>
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
    pub bind_port: u16,
    pub min_protocol_version: Option<String>,
    pub cipher_list: String,
    pub ssl_private_key_file: String,
    pub ssl_certificate_file: String
}

#[cfg(feature = "cgi")]
#[derive(Deserialize)]
pub struct CGI {
    pub enabled: bool,
    pub cgi_server: String,
    cgi_rules: HashMap<String, bool>
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "Config::default_max_content_length")]
    pub max_content_length: usize,
    pub global_response_headers: Option<HashMap<String, String>>,
    pub access_control: Option<AccessControl>,
    pub bind_host: String,
    pub bind_port: u16,
    pub endpoints: Option<Vec<String>>,
    pub endpoints_library: Option<String>,
    #[serde(default = "Config::default_cache_max_age")]
    pub cache_max_age: u64,
    pub encoding: Option<Encoding>,
    pub document_root: String,
    pub server_root: String,
    pub https: Option<Https>,
    #[cfg(target_family = "unix")]
    #[serde(default)]
    pub chroot: bool,
    #[serde(default)]
    pub enable_trace: bool,
    #[serde(default = "Config::default_server_header_state")]
    pub enable_server_header: bool,
    #[serde(default = "Config::default_request_timeout")]
    pub request_timeout: u64,
    #[serde(default)]
    pub be_verbose: bool,
    #[cfg(feature = "cgi")]
    pub cgi: Option<CGI>
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

    const fn default_max_content_length() -> usize {
        1073741824
    }

    const fn default_server_header_state() -> bool {
        true
    }

    const fn default_cache_max_age() -> u64 {
        3600
    }

    const fn default_request_timeout() -> u64 {
        10
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
        #[cfg(target_family = "unix")]
        let document_root = if *&*CHROOT {&String::from("")} else {&CONFIG.document_root};
        #[cfg(not(target_family = "unix"))]
        let document_root = &CONFIG.document_root;

        for (k, v) in &self.list {
            if let Ok(paths) = glob(&*format!("{document_root}/{k}")) {
                for entry in paths.filter_map(Result::ok) {
                    #[cfg(target_family = "unix")]
                    if entry.to_string_lossy().eq(&*format!("{document_root}/{resource}")) {
                        if v.eq("deny") {
                            return false;
                        }
                    }
                    #[cfg(not(target_family = "unix"))]
                    if entry.to_string_lossy().eq(&*format!("{document_root}\\{resource}")) {
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

impl Https {
    pub fn configure_ssl(&self) -> Result<SslContext, ErrorStack> {
        let server_root = &CONFIG.server_root;
        let mut ssl_ctx_builder = SslContext::builder(SslMethod::tls())?;

        ssl_ctx_builder.set_private_key_file(format!("{}/{}", server_root, &self.ssl_private_key_file), SslFiletype::PEM)?;
        ssl_ctx_builder.set_certificate_file(format!("{}/{}", server_root, &self.ssl_certificate_file), SslFiletype::PEM)?;
        ssl_ctx_builder.check_private_key()?;

        ssl_ctx_builder.set_min_proto_version(
            match &self.min_protocol_version {
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

        match ssl_ctx_builder.min_proto_version() {
            Some(SslVersion::TLS1_3) => {
                ssl_ctx_builder.set_ciphersuites(&self.cipher_list)?;
            },
            _ => {
                ssl_ctx_builder.set_cipher_list(&self.cipher_list)?;
            }
        }

        ssl_ctx_builder.set_verify(SslVerifyMode::PEER);
        ssl_ctx_builder.set_alpn_select_callback(|_ssl, client_protocols| {
            if let Some(p) = select_next_proto(b"\x08http/1.1", client_protocols) {
                Ok(p)
            } else {
                Err(AlpnError::ALERT_FATAL)
            }
        });

        ssl_ctx_builder.set_options(SslOptions::NO_TICKET);
        ssl_ctx_builder.set_session_cache_mode(SslSessionCacheMode::OFF);

        Ok(ssl_ctx_builder.build())
    }
}

#[cfg(feature = "cgi")]
impl CGI {
    pub fn should_attempt_cgi(&self, resource: &String) -> bool {
        #[cfg(target_family = "unix")]
        let document_root = if *&*CHROOT {&String::from("")} else {&CONFIG.document_root};
        #[cfg(not(target_family = "unix"))]
        let document_root = &CONFIG.document_root;

        for (k, v) in &self.cgi_rules {
            if let Ok(paths) = glob(&*format!("{document_root}/{k}")) {
                for entry in paths.filter_map(Result::ok) {
                    #[cfg(target_family = "unix")]
                    if entry.to_string_lossy().eq(&*format!("{document_root}/{resource}")) {
                        return *v;
                    }
                    #[cfg(not(target_family = "unix"))]
                    if entry.to_string_lossy().eq(&*format!("{document_root}\\{resource}")) {
                        return *v;
                    }
                }
            }
        }
        false
    }
}