use std::any::Any;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::LazyLock;
use drain_common::cookies::SetCookie;
use drain_common::RequestData;
use libloading::{Library, Error as LibError};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use crate::config::CONFIG;
use crate::pages::internal_server_error::internal_server_error;

type Endpoint = fn(RequestData,
                   &HashMap<String, String>,
                   &mut HashMap<String, String>,
                   &mut HashMap<String, SetCookie>,
                   &mut u16,
                   &String,
                   &IpAddr,
                   &u16,
                   &IpAddr,
                   &u16) -> Result<Option<Vec<u8>>, Box<dyn Any + Send>>;

pub static ENDPOINT_LIBRARY: LazyLock<Option<Library>> = LazyLock::new(|| {
    if let Some(endpoints_library) = &CONFIG.endpoints_library {
        println!("Initializing the library...");
        unsafe {
            return match Library::new(format!("{}/{}", &CONFIG.server_root, endpoints_library)) {
                Ok(lib) => {
                    println!("Success.{}", if CONFIG.be_verbose {"\r\nPUT, DELETE and PATCH are available."} else {""});
                    Some(lib)
                },
                Err(e) => {
                    eprintln!("[ENDPOINT_LIBRARY:{}] An error occurred while opening a dynamic library file. \
                                                     Check if dynamic_pages_library field in config.json is correct. Proceeding without it...\n\
                                                     Error information:\n{e}\n", line!());
                    if CONFIG.be_verbose {
                        println!("PUT, DELETE and PATCH are disabled.");
                    }
                    None
                }
            }
        }
    }

    println!("Library not provided, skipping...{}", if CONFIG.be_verbose {"\r\nPUT, DELETE and PATCH are disabled."} else {""});
    None
});

pub async fn endpoint<'a, T>(endpoint: &str,
                             stream: &mut T,
                             request_data: RequestData<'a>,
                             request_headers: &HashMap<String, String>,
                             response_headers: &mut HashMap<String, String>,
                             set_cookie: &mut HashMap<String, SetCookie>,
                             status: &mut u16,
                             local_ip: &IpAddr,
                             remote_ip: &IpAddr,
                             remote_port: &u16,
                             library: &Library) -> Result<Option<Vec<u8>>, LibError>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    match unsafe {
        let endpoint_symbol = String::from(endpoint).replace(|x| x == '/' || x == '\\', "::");
        let e = library.get::<Endpoint>(endpoint_symbol.as_bytes())?;

        e(request_data, &request_headers, response_headers, set_cookie, status, &CONFIG.bind_host, local_ip, &CONFIG.bind_port, remote_ip, remote_port)
    } {
        Ok(content) => Ok(content),
        Err(e) => {
            if let Some(e) = e.downcast_ref::<&str>() {
                eprintln!("[endpoint():{}] A panic occurred inside the dynamic endpoint. Error information:\n{e}", line!());
            } else if let Some(e) = e.downcast_ref::<String>() {
                eprintln!("[endpoint():{}] A panic occurred inside the dynamic endpoint. Error information:\n{e}", line!());
            } else {
                eprintln!("[endpoint():{}] A panic occurred inside the dynamic endpoint. No information about the error.", line!());
            }

            eprintln!("Attempting to send Internal Server Error page to the client...");
            if let Err(e) = internal_server_error(stream).await {
                eprintln!("[endpoint():{}] FAILED. Error information:\n{e}", line!());
            }
            eprintln!("Attempting to close connection...");
            if let Err(e) = stream.shutdown().await {
                eprintln!("[endpoint():{}] FAILED. Error information:\n{e}", line!());
            }
            panic!("Unrecoverable error occurred while handling connection.");
        }
    }
}