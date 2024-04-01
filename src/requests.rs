use std::collections::HashMap;
use tokio::io::ErrorKind;
use tokio::fs::*;
use tokio::net::TcpStream;
use crate::util::*;
use regex::*;
use crate::pages::select::select;
use crate::pages::not_found::not_found;
use crate::requests::RequestData::*;

pub enum Request {
    Get {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>},
    Head {resource: String, headers: HashMap<String, String>},
    Post {resource: String, headers: HashMap<String, String>, data: String},
    Put {resource: String, headers: HashMap<String, String>, data: String},
    Delete {resource: String, headers: HashMap<String, String>},
    Connect {resource: String, headers: HashMap<String, String>},
    Options {resource: String, headers: HashMap<String, String>},
    Trace {resource: String, headers: HashMap<String, String>},
    Patch {resource: String, headers: HashMap<String, String>, data: String},
}

pub enum RequestData<'a> {
    Get {params: &'a Option<HashMap<String, String>>, headers: &'a HashMap<String, String>},
    Post {headers: &'a HashMap<String, String>, data: &'a String},
    Head {headers: &'a HashMap<String, String>}
}

impl Request {
    pub fn parse_from_string(request_string: &String) -> Result<Self, ErrorKind> {
        let general_regex = Regex::new(
            r#"^((GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH) /((([A-Za-z0-9\-_]+\.[[:alnum:]]+)|([A-Za-z0-9\-_]+))(\?([[:alnum:]]+=[[:alnum:]]+)(&[[:alnum:]]+=[[:alnum:]]+)*)?)? (HTTP/((0\.9)|(1\.0)|(1\.1)|(2)|(3))))(\r\n(([[:alnum]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)))*[\S\s]*\z"#
        ).unwrap();

        if !general_regex.is_match(request_string.as_str()) {
            return Err(ErrorKind::InvalidInput);
        }

        let request_line = request_string.lines().next().unwrap();
        let mut iter_req_line = request_line.split_whitespace();

        let req_type = iter_req_line.next().unwrap().to_uppercase();
        let resource_with_params = iter_req_line.next().unwrap().trim();
        let mut resource = resource_with_params.to_string();
        let mut params: HashMap<String, String> = HashMap::new();
        if request_line.contains('?') {
            resource = resource_with_params[..resource_with_params.find('?').unwrap()].parse::<String>().unwrap();
            let params_str = resource_with_params[resource_with_params.find('?').unwrap() + 1..].parse::<String>().unwrap();

            for kv in params_str.split('&') {
                params.insert(kv[..kv.find('=').unwrap()].to_string(), kv[kv.find('=').unwrap() + 1..].to_string());
            }
        }

        let regex_headers = Regex::new(r#"^([[:alnum:]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)$"#).unwrap();

        let headers_iter = request_string.lines().skip(1).take_while(|x| { regex_headers.is_match(x)});
        let mut headers: HashMap<String, String> = HashMap::new();

        let data_iter = headers_iter.clone();

        for header in headers_iter {
            headers.insert(header[..header.find(':').unwrap()].to_string(), header[header.find(':').unwrap() + 1..].to_string());
        }

        let mut data = String::new();
        for line in data_iter {
            data.push_str(line);
        }

        let req = match req_type.trim() {
            "GET" => Self::Get {resource, params: if params.is_empty() {None} else {Some(params)}, headers},
            "HEAD" => Self::Head {resource, headers},
            "POST" => Self::Post {resource, headers, data},
            "PUT" => Self::Put {resource, headers, data},
            "DELETE" => Self::Delete {resource, headers},
            "CONNECT" => Self::Connect {resource, headers},
            "OPTIONS" => Self::Options {resource, headers},
            "TRACE" => Self::Trace {resource, headers},
            "PATCH" => Self::Patch {resource, headers, data},
            _ => return Err(ErrorKind::InvalidInput)
        };
        Ok(req)
    }
}

pub async fn handle_get(mut stream: TcpStream, resource: &String, parameters: &Option<HashMap<String, String>>, headers: &HashMap<String, String>) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    match resource_clone.as_str() {
        "select" => return Ok(select(&mut stream, Get {params: parameters, headers}).await?),
        _ => ()
    }

    if resource_clone.is_empty() {
        resource_clone = "index".to_string();
    }

    let file = File::open(resource_clone).await;

    let mut content = String::new();

    match file {
        Ok(mut f) => {
            let response_headers = HashMap::from([
                ("Connection", "keep-alive"),
                ("Keep-Alive", "timeout=5, max=100")]);

            read_to_string_wrapper(&mut f, &mut content, &mut stream, headers).await;
            send_response(&mut stream, 200, Some(response_headers), Some(content)).await?;
            Ok(())
        },
        Err(_) => {
            not_found(&mut stream, headers).await
        }
    }
}

pub async fn handle_head(mut stream: TcpStream, resource: &String, headers: &HashMap<String, String>) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    match resource_clone.as_str() {
        "select" => select(&mut stream, Head {headers}).await?,
        _ => {}
    }

    if resource_clone.is_empty() {
        resource_clone = "index".to_string();
    }

    let file = File::open(resource_clone).await;

    let mut content = String::new();

    match file {
        Ok(mut f) => {
            read_to_string_wrapper(&mut f, &mut content, &mut stream, headers).await;
            let content_length = content.len().to_string();

            let response_headers = HashMap::from([
                ("Connection", "keep-alive"),
                ("Keep-Alive", "timeout=5, max=100"),
                ("Content-Length", &*content_length)]);

            send_response(&mut stream, 200, Some(response_headers), None).await?;
            Ok(())
        },
        Err(_) => {
            not_found(&mut stream, headers).await
        }
    }
}

pub async fn handle_post(stream: TcpStream, resource: &String, headers: &HashMap<String, String>, data: &String) {

}

pub async fn handle_options(mut stream: TcpStream, resource: &String, headers: &HashMap<String, String>) -> Result<(), ErrorKind> {
    let response_headers = HashMap::from([("Accept", "GET, HEAD, POST, OPTIONS"),
                                                                ("Connection", "keep-alive"),
                                                                ("Keep-Alive", "timeout=5, max=100")]);
    send_response(&mut stream, 204, Some(response_headers), None).await?;
    Ok(())
}