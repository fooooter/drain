pub struct Config {
    pub db_url: &'static str
}

pub static CONFIG: Config = Config {
    db_url: "mariadb://localhost:3306/baza_testowa"
};