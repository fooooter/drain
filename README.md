[![crates.io](https://img.shields.io/badge/crates.io-v1.5.2-darkblue)](https://crates.io/crates/drain_server)

## Progress done so far (and TODO in the future)
[✔]   	GET<br>
[✔]   	OPTIONS<br>
[✔]   	HEAD<br>
[✔]   	POST<br>
[✔]   	TRACE<br>
[✔]   	PUT<br>
[✔]   	PATCH<br>
[✔]   	DELETE<br>
[✔]	Auto-detect MIME types<br>
[✔]	Cookies<br>
[✔]	Sessions<br>
[✔]	Config (now JSON)<br>
[✔]     Compression (GZIP and Brotli for now)<br>
[✔]     Decompression (GZIP and Brotli for now)<br>
[✔]     TLS<br>
[✔]	Redirections<br>
[✖]     HTTP/2<br>
[✖]     HTTP/3<br>
[✔]     CGI<br>
[✖]     Virtual hosting<br>


## About Drain

Dynamic pages/endpoints are generated inside a dynamic library, so that it's easy to create them without modifying
the core and recompiling the server only to change one thing on a page or inside the endpoint.

## OS compatibility

Drain should work properly under every POSIX-compatible OS like Linux (under which it has been tested on primarily), BSDs and macOS.
On Windows it works properly unless you have to use the SSL.

## Dependencies

### DISCLAIMER: It's discouraged to use crates.io package for stuff other than testing, because it's been provided with statically linked OpenSSL. Try to build Drain from GitHub repository when possible.

Currently only OpenSSL and libc.

## Build

To build Drain, run `cargo build` in the root of a source.

### CGI feature flag

In order to compile-in support for the CGI interface (for executing PHP scripts, for example), add `"cgi"` to the `default` field in Cargo.toml.

## Configuration

Drain can be configured using config.json file. In order to use a config.json file, you have to specify it in `DRAIN_CONFIG` environment variable.
Currently available fields are:

- `max_content_length` - maximum length of request's body. If exceeded, the server returns 413 status. Default is 1 GiB (1073741824 bytes).
- `global_response_headers` - it's a list of key-value pairs, which stand for default response headers appended to every
`response_headers` HashMap.
- `access_control`:
  * `list` - here you can control, which resources will be returned to the client and which won't through a list of key-value pairs. 
  In order to deny access to a resources matching the given pattern, type "deny" (default action is "allow").
  It uses Glob UNIX shell-like path syntax, so you can match extensions or even whole directories recursively!
  Directories are relative to `document_root`.
  * `deny_action` - it's an unsigned integer corresponding to either 404 or 403 HTTP status codes, which will be returned by the server alongside the 
  page corresponding to each status if access to the resource is denied. For safety reasons, the default is 404, so that a client won't
  know if the resource is unavailable or access to it is denied.
- `bind_host` - bind host to the server.
- `bind_port` - bind port to the server (HTTP). If you want to use 80, be sure to start the server as root or another privileged user.
- `endpoints` - holds a list of every dynamic page/endpoint available, so if you create one, be sure to specify it here!
- `endpoint_library` - a path to the dynamic library for dynamic pages/endpoints, which must be relative to the `server_root`.
- `cache_max_age` - max-age in `Cache-Control` header. Applied automatically only for static resources. Default is 3600 seconds (1 hour).
- `encoding`:
  * `use_encoding` - a name of encoding which will be used to compress the response body. It should be present in `supported_encodings`, otherwise the server will return uncompressed data.
  * `supported_encodings` - a list of all compression algorithms supported by the server. It can currently contain only "gzip" and "br".
  * `encoding_applicable_mime_types` - a list of media types to which encoding should be applied. It's best to leave this setting as is.
- `document_root` - a directory in which documents/files returned to the client are stored. Makes for the root of a URL.
- `server_root` - a directory in which server data are kept, like, for example, dynamic endpoint libraries.
- `index_of_page_rules` - here you can control, for which directories the "index of" page will be displayed when no index file is found, and for which won't through a list of key-value pairs.
  In order to have the server send "index of" page, when the directory matches the given pattern, set `true` (default action is `false`).
  It uses Glob UNIX shell-like path syntax, so you can match directories recursively!
  Directories are relative to `document_root`.
- `indices` - a list containing all index files, that the server will pick when the resource given by the client is a directory. Files are picked with respect to their order
  in this field.
- `https`:
  * `enabled` - enable HTTPS.
  * `bind_port` - bind port to the server (HTTPS). If you want to use 443, be sure to start the server as root or another privileged user.
  * `min_protocol_version` - a minimum version of TLS/DTLS/SSL the server accepts. Must be one of the following: 
    + SSL3
    + TLS1.3
    + TLS1
    + DTLS1
    + DTLS1.2
    + TLS1.1
    + TLS1.2
    
    Instead, it will be set to accept every protocol.
  * `cipher_list` - a colon-separated list of ciphers the server will use. Must be one of the following:
    + TLS_AES_128_GCM_SHA256
    + TLS_AES_256_GCM_SHA384
    + TLS_CHACHA20_POLY1305_SHA256
    + TLS_AES_128_CCM_SHA256
    + TLS_AES_128_CCM_8_SHA256
    + TLS_SHA384_SHA384 - integrity-only
    + TLS_SHA256_SHA256 - integrity-only
  
    Instead, the default configuration will be used: `TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:TLS_AES_128_GCM_SHA256`
  * `ssl_private_key_file` - a path to the private key file in PEM format (a necessary field once HTTPS is enabled).
  * `ssl_certificate_file` - a path to the certificate file in PEM format (a necessary field once HTTPS is enabled).
- `chroot` - whether to enable the chroot jail or not. False by default and available only in UNIX-like operating systems.
- `enable_trace` - whether to enable TRACE HTTP method or not. TRACE method is considered not very safe, so it's false by default 
  (when false, the server returns 405 status).
- `enable_server_header` - whether to enable the `Server` header or not. It contains "Drain " + its current version. True by default.
- `request_timeout` - a time the server will wait for data to be sent by the client; if it takes too long, the server will close the connection. Set to 10 seconds by default.
- `be_verbose` - toggle verbose output. False by default.
- `cgi` (CGI feature flag only!):
  * `enabled` - enable CGI in runtime.
  * `cgi_server` - a path to the application, which will process CGI requests (for example `php-cgi`)
  * `cgi_rules` - here you can control, of which resources the processing by the CGI server will be attempted, and for which won't through a list of key-value pairs.
  In order to have the CGI server process resources matching the given pattern, set `true` (default action is `false`). A resource is, for example, `index.php`.
  It uses Glob UNIX shell-like path syntax, so you can match extensions or even whole directories recursively!
  Directories are relative to `document_root`.

Drain must be restarted in order for changes to take effect.
Currently, the required fields are: `bind_host`, `bind_port`, `document_root` and `server_root`.

Exemplar config.json file is available in the root of this repository. Feel free to change it to your preferences.

PUT, DELETE and PATCH get enabled only when the library is loaded properly, otherwise these have no use, 
as it would be dangerous for the server to arbitrarily guess, what they should do in certain scenarios.
Therefore, they can be handled explicitly only inside the dynamic endpoint.

## Usage

### Chroot jail (UNIX-like OSes only)

Chroot jail functionality makes the whole operation of the server way more secure by setting the root directory
to the document root specified in config.json; but be wary, though, that files from outside the new root won't be 
accessible unless the directories containing them are mounted with the `--bind` flag into the document root. 

If you decide to mount anything, that shouldn't be sent by the server in response, please ensure that access to it is disabled with `access_control` in config.json.

Don't worry about SSL keys and endpoint library - they're loaded before the chroot.

### Template

It's strongly advised to use a template - https://github.com/fooooter/drain_page_template
(mainly because the default 404 and 403 pages are defined inside this template)

### Macros

Stick to the macro library if possible - https://github.com/fooooter/drain_macros

### `drain_common` library crate

Some things (i.e. cookies and sessions), that are mentioned from this point forward, are coming from `drain_common` crate - https://github.com/fooooter/drain_common

### Structure

Each endpoint should be a Rust module defined in a separate file, declared in lib.rs and have the following structure:

```rust
use drain_common::RequestData::*;
use drain_macros::*;

#[drain_endpoint("index")]
pub fn index() {
    let content: Vec<u8> = Vec::from(format!(r#"
        <!DOCTYPE html>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                <title>Index</title>
            </head>
            <body>
                Hello, world! {} request was sent.
            </body>
        </html>"#, match REQUEST_DATA {
        Get(_) => "GET",
        Post {..} => "POST",
        Head(_) => "HEAD",
        Put {..} => "PUT",
        Delete {..} => "DELETE",
        Patch {..} => "PATCH"
    }));
  
    set_header!("Content-Type", "text/html; charset=utf-8");
  
    Some(content)
}
```

Every variable referenced below is always present in the scope.
Though, async is not present in the function definition, the entire scope is asynchronous and every `Future` inside can be awaited.
It's using a Tokio runtime, so if you decide to import SQLx or other asynchronous crate, go ahead and specify Tokio in "features" in Cargo.toml.
The dynamic page ALWAYS returns `Option<Vec<u8>>`, no matter what you specify as a return type (it'll be ignored and `Option<Vec<u8>>` will be used regardless).

It's also possible to simulate directory structure using this `drain_endpoint` macro. 
Instead of `#[drain_endpoint("index")]` you can, for example, specify `#[drain_endpoint("settings/index")]`.
It will correspond to `/settings/index` URL path. Furthermore, the effect will be the same when you specify `/settings` in the URL.
Keep in mind you'd have to specify `settings/index` inside `endpoints` field in config.json.

### RequestData

`REQUEST_DATA` of the type `RequestData` is a struct-like Enum, which has variants, that tell, what kind of HTTP request method was used and stores
request headers and data specific to each variant.

```rust
pub enum RequestData<'a> {
    Get(&'a Option<HashMap<String, String>>),
    Post {data: &'a Option<RequestBody>, params: &'a Option<HashMap<String, String>>},
    Head(&'a Option<HashMap<String, String>>),
    Put {params: &'a Option<HashMap<String, String>>, data: &'a Option<RequestBody>},
    Delete {params: &'a Option<HashMap<String, String>>, data: &'a Option<RequestBody>},
    Patch {params: &'a Option<HashMap<String, String>>, data: &'a Option<RequestBody>},
    Default
}
```

POST, PUT, DELETE and PATCH `data` consists of a `RequestBody` enum, which contains data of a given media type. Currently supported request MIME types
are `application/x-www-form-urlencoded`, `multipart/form-data`, `plain/text` and `application/octet-stream` represented by 
`XWWWFormUrlEncoded`, `FormData`, `Plain` and `OctetStream` `RequestBody` enum variants respectively.

`Default` type is meant to be used primarily for handling `not_found` and `forbidden` pages when invoked outside of the regular request handlers, for example, during CGI.

`FormDataValue` is a struct containing possible filename of the data segment, its headers and its value. Keep in mind, that `value` is a `Vec<u8>`, because it can contain binary data, unlike
in `XWWWFormUrlEncoded`, where binary data are encoded using URL encoding.

```rust
pub struct FormDataValue {
    pub filename: Option<String>,
    pub value: Vec<u8>
}

pub enum RequestBody {
    XWWWFormUrlEncoded(HashMap<String, String>),
    FormData(HashMap<String, FormDataValue>),
    Plain(String),
    OctetStream(Vec<u8>)
}
```

`params` are regular key-value pairs sent in the URL represented by a HashMap. 

### Headers

`RESPONSE_HEADERS` is a HashMap containing every header field, that will be sent in response. It's a mutable reference,
so that you can simply append a header to existing ones. Its best use cases are redirections using `Location` header and
changing content type to JSON, for example. `Content-Type` header must be set explicitly, otherwise an empty page will be returned.

`REQUEST_HEADERS`, however, is a HashMap containing every header field, that was sent along with the request by the client.
You should use the `set_header!` and `header!` macros respectively, whenever possible.

### HTTP response codes

Drain provides an interface to manipulate HTTP response codes. 

There is a `&mut u16` variable `HTTP_STATUS_CODE` present 
in every scope inside a function marked as an endpoint, that can be dereferenced in order to set a status code that will be presented
to the client in response.

### Sessions

Drain's sessions enable users to store data, that are bound to a particular client, across requests, server-side.
To start a session, use `start_session!()` macro. It returns a `Session` struct, which contains the following methods:

- `async fn get<'a, V: SessionValue + Clone + 'static>(&'a self, k: &'a String) -> Option<V>` - it returns an object, 
that implements the `SessionValue` and `Clone` traits, based on a provided key `k`
- `async fn set(&mut self, k: String, v: Box<dyn SessionValue>)` - it sets the session field identified by `k` to the provided `v`,
which is a boxed `SessionValue` trait object
- `async fn destroy(mut self)` - destroys the session

Sessions are automatically cleared after about an hour.

Everything you want to put in the session has to implement the `SessionValue` and `Clone` traits.
You can do it using the `SessionValue` derive macro from `drain_macros`, as follows:

```rust
use drain_macros::SessionValue;

#[derive(SessionValue, Clone)]
struct Example;
```

Once a session is created, it sets the cookie `SESSION_ID` to a randomly-generated Base64 value.

### Cookies

`SET_COOKIE` is a HashMap containing every cookie to be set by the client. It's initially empty, and is also mutably referenced.

To get a HashMap of all cookies, you can use the `cookies!` macro. 
Keep in mind, that it returns `Option<HashMap<String, String>>`, so the result can be `None` if there are no cookies.
Cookies can be set by inserting a name of the cookie and a `SetCookie` struct into the `SET_COOKIE` HashMap.

`SetCookie` is defined as follows:
```rust
pub struct SetCookie {
    pub value: String,
    pub domain: Option<String>,
    pub expires: Option<String>,
    pub httponly: bool,
    pub max_age: Option<u32>,
    pub partitioned: bool,
    pub path: Option<String>,
    pub samesite: Option<SameSite>,
    pub secure: bool
}
```

### Redirections

Redirections are done once you append the `Location` header to `RESPONSE_HEADERS` in a dynamic page. 
It's up to you, whether a page should return content in redirection response or not, but it's preferred to 
return `None` after specifying `Location`. The status code is set by default to 302, but this will be changeable very soon.

### Client's IP and port

Client's IP and port can be obtained using `REMOTE_IP` (of the type `&IpAddr`) and `REMOTE_PORT` (of the type `&u16`) variables respectively inside the 
dynamic endpoint.

### Server's IP, hostname and port

Server's IP, hostname and port can be obtained using `LOCAL_IP` (of the type `&IpAddr`), `LOCAL_HOSTNAME` (of the type `&String`) and `LOCAL_PORT` (of the type `&u16`) variables
respectively inside the dynamic endpoint.