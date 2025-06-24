use std::collections::HashMap;
use std::error::Error;
use std::net::IpAddr;
use std::str::FromStr;
use drain_common::cookies::SetCookie;
use drain_common::RequestData;
use libloading::Library;
use mime_guess::Mime;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::config::CONFIG;
use crate::endpoints::endpoint;
use crate::util::ResourceType::Dynamic;
use crate::util::send_response;

pub async fn forbidden<'a, T>(stream: &mut T,
                          request_data: RequestData<'a>,
                          headers: &HashMap<String, String>,
                          mut response_headers: HashMap<String, String>,
                          local_ip: &IpAddr,
                          remote_ip: &IpAddr,
                          remote_port: &u16,
                          library: &Library) -> Result<(), Box<dyn Error + Send + Sync>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
    let content = endpoint(
        "forbidden",
        stream,
        request_data,
        headers,
        &mut response_headers,
        &mut set_cookie,
        &mut 403u16,
        local_ip,
        remote_ip,
        remote_port,
        library).await;
    let content_type = response_headers.get("content-type");

    if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
        let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
            (mime.to_string(), mime.type_().to_string())
        } else {
            response_headers.remove(&String::from("content-type"));
            return send_response(stream, 403, Some(response_headers), None, Some(set_cookie), None).await;
        };

        if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
            response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
            response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
        }

        return send_response(stream, 403, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
    }

    send_response(stream, 403, Some(response_headers), None, Some(set_cookie), None).await
}