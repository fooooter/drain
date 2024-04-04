use std::collections::HashMap;
use once_cell::sync::Lazy;

pub struct Config {
    pub db_url: &'static str,
    pub global_response_headers: Lazy<HashMap<&'static str, &'static str>>
}

pub static CONFIG: Config = Config {
    db_url: "mariadb://localhost:3306/baza_testowa",
    global_response_headers: Lazy::new(|| {HashMap::from([
        ("Connection", "keep-alive"),
        ("Keep-Alive", "timeout=5, max=100")
    ])})
};