use std::collections::HashMap;

mod not_found;
mod index;
mod forbidden;

pub enum RequestData<'a> {
    Get {params: &'a Option<HashMap<String, String>>, headers: &'a HashMap<String, String>},
    Post {headers: &'a HashMap<String, String>, data: &'a Option<HashMap<String, String>>},
    Head {headers: &'a HashMap<String, String>}
}