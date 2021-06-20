#![allow(clippy::unwrap_used)]

use std::{collections::HashMap, sync::Arc};

use async_lock::Mutex;
use async_std::prelude::*;
use chrono::{Duration, Utc};
use once_cell::sync::Lazy;
use openidconnect::{core::CoreIdTokenClaims, HttpRequest, HttpResponse};
use tide::{
    http::headers::{COOKIE, LOCATION, SET_COOKIE},
    sessions::{MemoryStore, SessionMiddleware},
    Request, StatusCode,
};
use tide_testing::TideTestingExt;

use crate::{
    ClientId, ClientSecret, IssuerUrl, OpenIdConnectMiddleware, OpenIdConnectRequestExt,
    OpenIdConnectRouteExt, RedirectUrl,
};

const SECRET: [u8; 32] = *b"secrets must be >= 32 bytes long";

static ISSUER_URL: Lazy<IssuerUrl> =
    Lazy::new(|| IssuerUrl::new("https://localhost/issuer_url".to_string()).unwrap());
static CLIENT_ID: Lazy<ClientId> = Lazy::new(|| ClientId::new("CLIENT-ID".to_string()));
static CLIENT_SECRET: Lazy<ClientSecret> =
    Lazy::new(|| ClientSecret::new("CLIENT-SECRET".to_string()));
static REDIRECT_URL: Lazy<RedirectUrl> =
    Lazy::new(|| RedirectUrl::new("https://localhost/callback".to_string()).unwrap());

fn get_tidesid_cookie(response: &tide_testing::surf::Response) -> tide::http::Cookie {
    tide::http::Cookie::parse(
        response
            .header(SET_COOKIE)
            .unwrap()
            .get(0)
            .unwrap()
            .to_string(),
    )
    .unwrap()
}

#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum Error {
    // /// Test error.
// #[error("Test error: {}", _0)]
// Test(String),
}

type PendingResponse = (String, Result<HttpResponse, Error>);

task_local! {
    static PENDING_RESPONSE: Arc<Mutex<Vec<PendingResponse>>> =
        Arc::new(Mutex::new(vec![]));
}

async fn set_pending_response(response: Vec<PendingResponse>) {
    let pending_response_guard = PENDING_RESPONSE.with(|pr| pr.clone());
    let mut pending_response = pending_response_guard.lock().await;
    *pending_response = response;
}

async fn pending_response_is_empty() -> bool {
    let pending_response_guard = PENDING_RESPONSE.with(|pr| pr.clone());
    let pending_response = pending_response_guard.lock().await;
    pending_response.is_empty()
}

pub(crate) async fn http_client(openid_request: HttpRequest) -> Result<HttpResponse, Error> {
    // Get the pending response, which must exist (otherwise the test
    // has a bug).
    let pending_response_guard = PENDING_RESPONSE.with(|pr| pr.clone());
    let mut pending_response = pending_response_guard.lock().await;

    // Pop the first request from the vector, *ensure that it matches
    // the request URI,* then return that response.
    if pending_response.is_empty() {
        panic!("No pending response for URL \"{}\"", openid_request.url);
    }
    let (expected_uri, response) = pending_response.remove(0);
    assert_eq!(openid_request.url.to_string(), expected_uri);
    response
}

fn create_discovery_response() -> PendingResponse {
    (
        "https://localhost/issuer_url/.well-known/openid-configuration".to_string(),
        Ok(HttpResponse {
            status_code: http::StatusCode::OK,
            headers: http::HeaderMap::new(),
            body: "{
                \"issuer\":\"https://localhost/issuer_url\",
                \"authorization_endpoint\":\"https://localhost/authorization\",
                \"token_endpoint\":\"https://localhost/token\",
                \"jwks_uri\":\"https://localhost/jwks\",
                \"response_types_supported\":[\"code\"],
                \"subject_types_supported\":[\"public\"],
                \"id_token_signing_alg_values_supported\":[\"RS256\"]
            }"
            .as_bytes()
            .into(),
        }),
    )
}

// From here: <https://github.com/ramosbugs/openidconnect-rs/blob/cfa5af581ee100791f68bf099dd15fa3eb492c8b/src/jwt.rs#L489>
const TEST_RSA_PUB_KEY: &str = "{
            \"kty\": \"RSA\",
            \"kid\": \"bilbo.baggins@hobbiton.example\",
            \"use\": \"sig\",
            \"n\": \"n4EPtAOCc9AlkeQHPzHStgAbgs7bTZLwUBZdR8_KuKPEHLd4rHVTeT\
                     -O-XV2jRojdNhxJWTDvNd7nqQ0VEiZQHz_AJmSCpMaJMRBSFKrKb2wqV\
                     wGU_NsYOYL-QtiWN2lbzcEe6XC0dApr5ydQLrHqkHHig3RBordaZ6Aj-\
                     oBHqFEHYpPe7Tpe-OfVfHd1E6cS6M1FZcD1NNLYD5lFHpPI9bTwJlsde\
                     3uhGqC0ZCuEHg8lhzwOHrtIQbS0FVbb9k3-tVTU4fg_3L_vniUFAKwuC\
                     LqKnS2BYwdq_mzSnbLY7h_qixoR7jig3__kRhuaxwUkRz5iaiQkqgc5g\
                     HdrNP5zw\",
            \"e\": \"AQAB\"
        }";

const TEST_RSA_PRIV_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
         MIIEowIBAAKCAQEAn4EPtAOCc9AlkeQHPzHStgAbgs7bTZLwUBZdR8/KuKPEHLd4\n\
         rHVTeT+O+XV2jRojdNhxJWTDvNd7nqQ0VEiZQHz/AJmSCpMaJMRBSFKrKb2wqVwG\n\
         U/NsYOYL+QtiWN2lbzcEe6XC0dApr5ydQLrHqkHHig3RBordaZ6Aj+oBHqFEHYpP\n\
         e7Tpe+OfVfHd1E6cS6M1FZcD1NNLYD5lFHpPI9bTwJlsde3uhGqC0ZCuEHg8lhzw\n\
         OHrtIQbS0FVbb9k3+tVTU4fg/3L/vniUFAKwuCLqKnS2BYwdq/mzSnbLY7h/qixo\n\
         R7jig3//kRhuaxwUkRz5iaiQkqgc5gHdrNP5zwIDAQABAoIBAG1lAvQfhBUSKPJK\n\
         Rn4dGbshj7zDSr2FjbQf4pIh/ZNtHk/jtavyO/HomZKV8V0NFExLNi7DUUvvLiW7\n\
         0PgNYq5MDEjJCtSd10xoHa4QpLvYEZXWO7DQPwCmRofkOutf+NqyDS0QnvFvp2d+\n\
         Lov6jn5C5yvUFgw6qWiLAPmzMFlkgxbtjFAWMJB0zBMy2BqjntOJ6KnqtYRMQUxw\n\
         TgXZDF4rhYVKtQVOpfg6hIlsaoPNrF7dofizJ099OOgDmCaEYqM++bUlEHxgrIVk\n\
         wZz+bg43dfJCocr9O5YX0iXaz3TOT5cpdtYbBX+C/5hwrqBWru4HbD3xz8cY1TnD\n\
         qQa0M8ECgYEA3Slxg/DwTXJcb6095RoXygQCAZ5RnAvZlno1yhHtnUex/fp7AZ/9\n\
         nRaO7HX/+SFfGQeutao2TDjDAWU4Vupk8rw9JR0AzZ0N2fvuIAmr/WCsmGpeNqQn\n\
         ev1T7IyEsnh8UMt+n5CafhkikzhEsrmndH6LxOrvRJlsPp6Zv8bUq0kCgYEAuKE2\n\
         dh+cTf6ERF4k4e/jy78GfPYUIaUyoSSJuBzp3Cubk3OCqs6grT8bR/cu0Dm1MZwW\n\
         mtdqDyI95HrUeq3MP15vMMON8lHTeZu2lmKvwqW7anV5UzhM1iZ7z4yMkuUwFWoB\n\
         vyY898EXvRD+hdqRxHlSqAZ192zB3pVFJ0s7pFcCgYAHw9W9eS8muPYv4ZhDu/fL\n\
         2vorDmD1JqFcHCxZTOnX1NWWAj5hXzmrU0hvWvFC0P4ixddHf5Nqd6+5E9G3k4E5\n\
         2IwZCnylu3bqCWNh8pT8T3Gf5FQsfPT5530T2BcsoPhUaeCnP499D+rb2mTnFYeg\n\
         mnTT1B/Ue8KGLFFfn16GKQKBgAiw5gxnbocpXPaO6/OKxFFZ+6c0OjxfN2PogWce\n\
         TU/k6ZzmShdaRKwDFXisxRJeNQ5Rx6qgS0jNFtbDhW8E8WFmQ5urCOqIOYk28EBi\n\
         At4JySm4v+5P7yYBh8B8YD2l9j57z/s8hJAxEbn/q8uHP2ddQqvQKgtsni+pHSk9\n\
         XGBfAoGBANz4qr10DdM8DHhPrAb2YItvPVz/VwkBd1Vqj8zCpyIEKe/07oKOvjWQ\n\
         SgkLDH9x2hBgY01SbP43CvPk0V72invu2TGkI/FXwXWJLLG7tDSgw4YyfhrYrHmg\n\
         1Vre3XB9HH8MYBVB6UIexaAq4xSeoemRKTBesZro7OKjKT8/GmiO\
         -----END RSA PRIVATE KEY-----";

fn create_jwks_response() -> PendingResponse {
    (
        "https://localhost/jwks".to_string(),
        Ok(HttpResponse {
            status_code: http::StatusCode::OK,
            headers: http::HeaderMap::new(),
            body: format!("{{\"keys\":[{}]}}", TEST_RSA_PUB_KEY)
                .as_bytes()
                .into(),
        }),
    )
}

#[derive(Debug, PartialEq)]
struct ParsedAuthorizeUrl {
    host: String,
    path: String,
    response_type: String,
    client_id: String,
    scopes: String,
    state: Option<String>,
    nonce: Option<String>,
    redirect_uri: String,
}

impl ParsedAuthorizeUrl {
    fn default() -> Self {
        Self {
            host: "localhost".to_owned(),
            path: "/authorization".to_owned(),
            response_type: "code".to_owned(),
            client_id: CLIENT_ID.to_string(),
            scopes: "openid".to_owned(),
            state: None,
            nonce: None,
            redirect_uri: "https://localhost/callback".to_string(),
        }
    }

    fn from_url(s: impl AsRef<str>) -> Self {
        let url = openidconnect::url::Url::parse(s.as_ref()).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();

        Self {
            host: url.host_str().unwrap().to_owned(),
            path: url.path().to_owned(),
            response_type: query.get("response_type").unwrap().to_owned(),
            client_id: query.get("client_id").unwrap().to_owned(),
            scopes: query.get("scope").unwrap().to_owned(),
            state: Some(query.get("state").unwrap().to_owned()),
            nonce: Some(query.get("nonce").unwrap().to_owned()),
            redirect_uri: query.get("redirect_uri").unwrap().to_owned(),
        }
    }

    fn with_nonce(self, nonce: Option<String>) -> Self {
        Self { nonce, ..self }
    }

    fn with_scopes(self, scopes: impl AsRef<str>) -> Self {
        Self {
            scopes: scopes.as_ref().to_owned(),
            ..self
        }
    }

    fn with_state(self, state: Option<String>) -> Self {
        Self { state, ..self }
    }
}

#[async_std::test]
async fn middleware_can_be_initialized() -> tide::Result<()> {
    set_pending_response(vec![create_discovery_response(), create_jwks_response()]).await;

    OpenIdConnectMiddleware::new(&ISSUER_URL, &CLIENT_ID, &CLIENT_SECRET, &REDIRECT_URL).await;

    assert!(pending_response_is_empty().await);

    Ok(())
}

#[async_std::test]
async fn middleware_provides_login_route() -> tide::Result<()> {
    let mut app = tide::new();
    app.with(
        SessionMiddleware::new(MemoryStore::new(), &SECRET)
            .with_same_site_policy(tide::http::cookies::SameSite::Lax),
    );

    set_pending_response(vec![create_discovery_response(), create_jwks_response()]).await;
    app.with(
        OpenIdConnectMiddleware::new(&ISSUER_URL, &CLIENT_ID, &CLIENT_SECRET, &REDIRECT_URL).await,
    );

    let res = app.get("/login").await?;
    assert_eq!(res.status(), StatusCode::Found);
    assert_eq!(
        ParsedAuthorizeUrl::from_url(res.header(LOCATION).unwrap().get(0).unwrap().as_str())
            .with_nonce(None)
            .with_state(None),
        ParsedAuthorizeUrl::default(),
    );

    Ok(())
}

#[async_std::test]
async fn login_path_can_be_changed() -> tide::Result<()> {
    let mut app = tide::new();
    app.with(
        SessionMiddleware::new(MemoryStore::new(), &SECRET)
            .with_same_site_policy(tide::http::cookies::SameSite::Lax),
    );

    set_pending_response(vec![create_discovery_response(), create_jwks_response()]).await;
    app.with(
        OpenIdConnectMiddleware::new(&ISSUER_URL, &CLIENT_ID, &CLIENT_SECRET, &REDIRECT_URL)
            .await
            .with_login_path("/oauthlogin"),
    );

    let res = app.get("/oauthlogin").await?;
    assert_eq!(res.status(), StatusCode::Found);
    assert_eq!(
        ParsedAuthorizeUrl::from_url(res.header(LOCATION).unwrap().get(0).unwrap().as_str())
            .with_nonce(None)
            .with_state(None),
        ParsedAuthorizeUrl::default(),
    );

    Ok(())
}

#[async_std::test]
async fn oauth_scopes_can_be_changed() -> tide::Result<()> {
    let mut app = tide::new();
    app.with(
        SessionMiddleware::new(MemoryStore::new(), &SECRET)
            .with_same_site_policy(tide::http::cookies::SameSite::Lax),
    );

    set_pending_response(vec![create_discovery_response(), create_jwks_response()]).await;
    app.with(
        OpenIdConnectMiddleware::new(&ISSUER_URL, &CLIENT_ID, &CLIENT_SECRET, &REDIRECT_URL)
            .await
            .with_scopes(&["profile"]),
    );

    let res = app.get("/login").await?;
    assert_eq!(res.status(), StatusCode::Found);
    assert_eq!(
        ParsedAuthorizeUrl::from_url(res.header(LOCATION).unwrap().get(0).unwrap().as_str())
            .with_nonce(None)
            .with_state(None),
        ParsedAuthorizeUrl::default().with_scopes("openid profile"),
    );

    Ok(())
}

#[async_std::test]
#[should_panic(
    expected = "request session not initialized, did you enable tide::sessions::SessionMiddleware?"
)]
async fn login_panics_on_missing_session_middleware() {
    let mut app = tide::new();
    // Note: *No* session middleware was added to the server.

    set_pending_response(vec![create_discovery_response(), create_jwks_response()]).await;
    app.with(
        OpenIdConnectMiddleware::new(&ISSUER_URL, &CLIENT_ID, &CLIENT_SECRET, &REDIRECT_URL).await,
    );

    let _result = app.get("/login").await;
}

// Request to redirect_url (with the authorization code and stuff): checks the nonce and CSRF, makes the token call, sets session state, can get req.user_id() or whatever.
#[async_std::test]
async fn middleware_provides_redirect_route() -> tide::Result<()> {
    let mut app = tide::new();
    app.with(
        SessionMiddleware::new(MemoryStore::new(), &SECRET)
            .with_same_site_policy(tide::http::cookies::SameSite::Lax),
    );

    set_pending_response(vec![create_discovery_response(), create_jwks_response()]).await;
    app.with(
        OpenIdConnectMiddleware::new(&ISSUER_URL, &CLIENT_ID, &CLIENT_SECRET, &REDIRECT_URL).await,
    );

    // Add the `/` route, which we use to check the authed/unauthed status
    // status of the request. Note that we would also like to use this
    // handler to confirm that session state is preserved across the auth
    // procedure (which it is), but that behavior is mostly a side-effect
    // of the SessionMiddleware's cookie setting (which *this* middleware
    // requires be set to Lax). So we could test that, but A) we would
    // only be testing that we set that to Lax earlier in this test function,
    // and more importantly B) wouldn't actually be testing the SameSite
    // setting anyway, since we are manually flowing the session cookie
    // across requests without even paying attention to the SameSite attribute
    // in the response.
    app.at("/").get(|req: Request<()>| async move {
        Ok(match req.user_id() {
            Some(userid) => format!("authed {}", userid),
            None => "unauthed".to_string(),
        })
    });

    let mut res = app.get("/").await?;
    assert_eq!(res.status(), StatusCode::Ok);
    assert_eq!(res.body_string().await?, "unauthed");

    let res = app.get("/login").await?;
    assert_eq!(res.status(), StatusCode::Found);
    let authorize_url =
        ParsedAuthorizeUrl::from_url(res.header(LOCATION).unwrap().get(0).unwrap().as_str());
    let state = authorize_url.state.clone().unwrap().to_string();
    let nonce = authorize_url.nonce.clone().unwrap();
    assert_eq!(
        authorize_url.with_nonce(None).with_state(None),
        ParsedAuthorizeUrl::default(),
    );
    // TODO Can we automate this somehow? Create some helper that auto-flows
    // the tide.sid cookie across requests? Maybe we just manually reimplement
    // tide-testing here, but have it auto-flow cookies if you provide the
    // previous/initial response... Maybe we do this using middleware, per
    // [this issue](https://github.com/http-rs/surf/issues/19)? In our case,
    // we replace tide-testing with our own custom thing, that uses middleware
    // for the cookie jar. Basically you: create the client (attached to a
    // server), add the cookie jar middleware, then send requests with that
    // client, which accumulate and send cookies with each request.
    let session_cookie: tide::http::Cookie = get_tidesid_cookie(&res);

    // TODO Factor this entire block out into a `create_id_token` function.
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new("https://localhost/issuer_url".to_string()).unwrap(),
        vec![openidconnect::Audience::new(CLIENT_ID.to_string())],
        Utc::now().checked_add_signed(Duration::hours(1)).unwrap(),
        Utc::now(),
        openidconnect::StandardClaims::new(openidconnect::SubjectIdentifier::new(
            "1234567890".to_string(),
        )),
        openidconnect::EmptyAdditionalClaims {},
    )
    .set_nonce(Some(openidconnect::Nonce::new(nonce)));

    let id_token = openidconnect::core::CoreIdToken::new(
        claims,
        &openidconnect::core::CoreRsaPrivateSigningKey::from_pem(TEST_RSA_PRIV_KEY, None).unwrap(),
        openidconnect::core::CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .unwrap();

    // TODO Factor this out into an `create_id_token_response` function.
    set_pending_response(vec![(
        "https://localhost/token".to_string(),
        Ok(HttpResponse {
            status_code: http::StatusCode::OK,
            headers: http::HeaderMap::new(),
            body: format!(
                "{{
                    \"access_token\":\"immatoken\",
                    \"token_type\":\"bearer\",
                    \"id_token\":{}
                }}",
                serde_json::to_string(&id_token).unwrap()
            )
            .as_bytes()
            .into(),
        }),
    )])
    .await;

    let res = app
        .get(format!("/callback?code=12345&state={}", state))
        .header(COOKIE, session_cookie.to_string())
        .await?;
    assert_eq!(res.status(), StatusCode::Found);
    assert_eq!(res.header(LOCATION).unwrap().get(0).unwrap(), "/");

    let mut res = app
        .get("/")
        .header(COOKIE, session_cookie.to_string())
        .await?;
    assert_eq!(res.status(), StatusCode::Ok);
    assert_eq!(res.body_string().await?, "authed 1234567890");

    Ok(())
}

// async fn redirect_route_rejects_invalid_csrf() -> tide::Result<()> {
// Same as above but with a non-matching CSRF: error.

// async fn redirect_route_rejects_invalid_nonce() -> tide::Result<()> {
// Same as above but with a non-matching nonce: error.

// async fn redirect_route_errors_on_missing_session_middleware() -> tide::Result<()> {
// *Error* (not panic) on missing session middleware, since this is indistinguishable from an expired session that was simply not present in the session store.
// I *think.* Let's verify that this is in fact what happens, because maybe we want one version that panics (if we can in fact detect that the session middleware is missing).

// TODO Add a big separator block here to show that we are now testing the route extensions.
// async fn unauthenticated_routes_do_not_force_login() -> tide::Result<()> {
// Basically: a request to a random /foo URL works.

#[async_std::test]
async fn authenticated_routes_require_login() -> tide::Result<()> {
    let mut app = tide::new();
    app.with(
        SessionMiddleware::new(MemoryStore::new(), &SECRET)
            .with_same_site_policy(tide::http::cookies::SameSite::Lax),
    );

    set_pending_response(vec![create_discovery_response(), create_jwks_response()]).await;
    app.with(
        OpenIdConnectMiddleware::new(&ISSUER_URL, &CLIENT_ID, &CLIENT_SECRET, &REDIRECT_URL).await,
    );

    app.at("/needsauth")
        .authenticated()
        .get(|_req: Request<()>| -> std::pin::Pin<Box<dyn Future<Output = tide::Result> + Send>> {
            panic!(
                "An unauthenticated request should not have made it to an `authenticated()` handler."
            );
        });

    let res = app.get("/needsauth").await?;
    assert_eq!(res.status(), StatusCode::Found);
    assert_eq!(
        res.header(LOCATION).unwrap().get(0).unwrap().to_string(),
        "/login"
    );

    Ok(())
}

// async fn authenticated_and_unauthenticated_routes_can_coexist() -> tide::Result<()> {
// Basically: two routes, one that works and one that redirects to /login.