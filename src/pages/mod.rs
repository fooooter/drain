pub mod internal_server_error;
pub mod index_of;

#[cfg(feature = "cgi")]
pub mod bad_gateway;
pub mod not_found;
pub mod forbidden;