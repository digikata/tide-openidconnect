use dotenv::dotenv;
use serde::Deserialize;
use tide_openidconnect::{self, OpenIdConnectRequestExt, OpenIdConnectRouteExt};

#[async_std::main]
async fn main() -> tide::Result<()> {
    dotenv().ok();
    let cfg = Config::from_env().unwrap();

    tide::log::with_level(tide::log::LevelFilter::Info);
    let mut app = tide::new();

    // app.with(tide_csrf::CsrfMiddleware::new(&SECRET));

    app.with(
        tide::sessions::SessionMiddleware::new(
            tide::sessions::MemoryStore::new(),
            cfg.tide_secret.as_bytes(),
        )
        .with_same_site_policy(tide::http::cookies::SameSite::Lax),
    );

    app.with(tide_openidconnect::OpenIdConnectMiddleware::new(&cfg.azure).await);

    app.at("/").authenticated().get(|req: tide::Request<()>| async move {
        // Use the access token to fetch the user's profile from Microsoft
        // Graph, then return a text response with that information.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ProfileResponse {
            display_name: String,
        }
        let ProfileResponse { display_name } = surf::get("https://graph.microsoft.com/v1.0/me")
            .header("Authorization", format!("Bearer {}", req.access_token().unwrap()))
            .recv_json()
            .await?;

        Ok(format!("This authenticated route allows me to access basic information from Microsoft Graph, such as your display name: {} (you have scopes {:?})", display_name, req.scopes().unwrap()))
    });

    app.listen("127.0.0.1:8000").await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct Config {
    tide_secret: String,
    azure: tide_openidconnect::Config,
}

impl Config {
    pub fn from_env() -> Result<Self, config::ConfigError> {
        let mut cfg = config::Config::new();
        cfg.merge(config::Environment::new().separator("__"))?;
        cfg.try_into()
    }
}
