use std::collections::HashMap;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub global_response_headers: HashMap<String, String>
}