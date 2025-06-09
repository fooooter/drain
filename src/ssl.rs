use std::sync::LazyLock;
use openssl::ssl::SslContext;
use crate::config::CONFIG;

pub struct SslInfo {
    pub ctx: SslContext,
    pub port: u16
}

pub static SSL: LazyLock<Option<SslInfo>> = LazyLock::new(|| {
    match &CONFIG.https {
        Some(https) if https.enabled => {
            match https.configure_ssl() {
                Ok(ctx) => {
                    println!("SSL enabled.");
                    return Some(SslInfo {ctx, port: https.bind_port})
                },
                Err(e) => {
                    eprintln!("[SSL:{}] An error occurred while configuring SSL for a secure connection.\n\
                                        Error information:\n{e}", line!());
                }
            }
        },
        _ => {}
    }

    println!("SSL disabled.");
    None
});