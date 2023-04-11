#![warn(clippy::future_not_send)]
#![cfg_attr(test, feature(lazy_cell))]

use std::{
    borrow::Borrow,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use http_client::{
    http_types::{headers, Method, StatusCode, Url},
    Body, HttpClient, Request, Response,
};
pub mod model;
use serde::Serialize;
use serde_with::skip_serializing_none;
use tap::TapFallible;
use tracing::{debug, trace};

use crate::{
    ext::*,
    model::{
        AddTorrentArg, BuildInfo, Category, Credential, GetLogsArg, GetTorrentListArg, HashArg,
        Hashes, HashesArg, Log, PeerLog, PeerSyncData, PieceState, Preferences, Priority, Sep,
        SetTorrentSharedLimitArg, SyncData, Torrent, TorrentContent, TorrentProperty,
        TorrentSource, Tracker, TransferInfo, WebSeed,
    },
};

mod ext;

pub struct Api<C> {
    client: C,
    endpoint: Url,
    credential: Credential,
    cookie: OnceLock<String>,
}

impl<C: HttpClient> Api<C> {
    pub fn new(endpoint: Url, credential: Credential, client: C) -> Self {
        Self {
            client,
            endpoint,
            credential,
            cookie: OnceLock::new(),
        }
    }

    pub fn new_with_cookie(endpoint: Url, cookie: String, client: C) -> Self {
        Self {
            client,
            endpoint,
            credential: Credential {
                username: String::new(),
                password: String::new(),
            },
            cookie: OnceLock::from(cookie),
        }
    }

    pub async fn get_cookie(&self) -> Result<Option<String>> {
        Ok(self.cookie.get().cloned())
    }

    pub async fn logout(&self) -> Result<()> {
        self.get("auth/logout", NONE).await.map(|_| ())
    }

    pub async fn get_version(&self) -> Result<String> {
        self.get("app/version", NONE)
            .await?
            .body_string()
            .await
            .map_err(Into::into)
    }

    pub async fn get_webapi_version(&self) -> Result<String> {
        self.get("app/webapiVersion", NONE)
            .await?
            .body_string()
            .await
            .map_err(Into::into)
    }

    pub async fn get_build_info(&self) -> Result<BuildInfo> {
        self.get("app/buildInfo", NONE)
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_preferences(&self) -> Result<Preferences> {
        self.get("app/preferences", NONE)
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn set_preferences(
        &self,
        preferences: impl Borrow<Preferences> + Send + Sync,
    ) -> Result<()> {
        self.post("app/setPreferences", NONE, Some(preferences.borrow()))
            .await
            .map_err(Into::into)
            .map(|_| ())
    }

    pub async fn get_default_save_path(&self) -> Result<PathBuf> {
        self.get("app/defaultSavePath", NONE)
            .await?
            .body_string()
            .await
            .map_err(Into::into)
            .map(PathBuf::from)
    }

    pub async fn get_logs(&self, arg: impl Borrow<GetLogsArg> + Send + Sync) -> Result<Vec<Log>> {
        self.get("log/main", Some(arg.borrow()))
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_peer_logs(
        &self,
        last_known_id: impl Into<Option<i64>> + Send + Sync,
    ) -> Result<Vec<PeerLog>> {
        #[derive(Serialize)]
        #[skip_serializing_none]
        struct Arg {
            last_known_id: Option<i64>,
        }

        self.get(
            "log/peers",
            Some(&Arg {
                last_known_id: last_known_id.into(),
            }),
        )
        .await?
        .body_json()
        .await
        .map_err(Into::into)
    }

    pub async fn sync(&self, rid: impl Into<Option<i64>> + Send + Sync) -> Result<SyncData> {
        #[derive(Serialize)]
        #[skip_serializing_none]
        struct Arg {
            rid: Option<i64>,
        }

        self.get("sync/maindata", Some(&Arg { rid: rid.into() }))
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_torrent_peers(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        rid: impl Into<Option<i64>> + Send + Sync,
    ) -> Result<PeerSyncData> {
        #[derive(Serialize)]
        struct Arg<'a> {
            hash: &'a str,
            rid: Option<i64>,
        }

        self.get(
            "sync/torrentPeers",
            Some(&Arg {
                hash: hash.as_ref(),
                rid: rid.into(),
            }),
        )
        .await
        .and_then(|r| r.map_status(TORRENT_NOT_FOUND))?
        .body_json()
        .await
        .map_err(Into::into)
    }

    pub async fn get_transfer_info(&self) -> Result<TransferInfo> {
        self.get("transfer/info", NONE)
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_speed_limits_mode(&self) -> Result<bool> {
        self.get("transfer/speedLimitsMode", NONE)
            .await?
            .body_string()
            .await
            .map_err(Into::into)
            .and_then(|s| match s.as_str() {
                "0" => Ok(false),
                "1" => Ok(true),
                _ => Err(Error::BadResponse {
                    explain: "Received non-number response body on `transfer/speedLimitsMode`",
                }),
            })
    }

    pub async fn toggle_speed_limits_mode(&self) -> Result<()> {
        self.get("transfer/toggleSpeedLimitsMode", NONE)
            .await
            .map(|_| ())
    }

    pub async fn get_download_limit(&self) -> Result<u64> {
        self.get("transfer/downloadLimit", NONE)
            .await?
            .body_string()
            .await
            .map_err(Into::into)
            .and_then(|s| {
                s.parse().map_err(|_| Error::BadResponse {
                    explain: "Received non-number response body on `transfer/downloadLimit`",
                })
            })
    }

    pub async fn set_download_limit(&self, limit: u64) -> Result<()> {
        #[derive(Serialize)]
        struct Arg {
            limit: u64,
        }

        self.get("transfer/setDownloadLimit", Some(&Arg { limit }))
            .await
            .map(|_| ())
    }

    pub async fn get_upload_limit(&self) -> Result<u64> {
        self.get("transfer/uploadLimit", NONE)
            .await?
            .body_string()
            .await
            .map_err(Into::into)
            .and_then(|s| {
                s.parse().map_err(|_| Error::BadResponse {
                    explain: "Received non-number response body on `transfer/uploadLimit`",
                })
            })
    }

    pub async fn set_upload_limit(&self, limit: u64) -> Result<()> {
        #[derive(Serialize)]
        struct Arg {
            limit: u64,
        }

        self.get("transfer/setUploadLimit", Some(&Arg { limit }))
            .await
            .map(|_| ())
    }

    pub async fn ban_peers(&self, peers: impl Into<Sep<String, '|'>> + Send + Sync) -> Result<()> {
        #[derive(Serialize)]
        struct Arg {
            peers: String,
        }

        self.get(
            "transfer/banPeers",
            Some(&Arg {
                peers: peers.into().to_string(),
            }),
        )
        .await
        .map(|_| ())
    }

    pub async fn get_torrent_list(&self, arg: GetTorrentListArg) -> Result<Vec<Torrent>> {
        self.get("torrents/info", Some(&arg))
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_torrent_properties(
        &self,
        hash: impl AsRef<str> + Sync + Send + Sync,
    ) -> Result<TorrentProperty> {
        self.get("torrents/properties", Some(&HashArg::new(hash.as_ref())))
            .await
            .and_then(|r| r.map_status(TORRENT_NOT_FOUND))?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_torrent_trackers(
        &self,
        hash: impl AsRef<str> + Send + Sync,
    ) -> Result<Vec<Tracker>> {
        self.get("torrents/trackers", Some(&HashArg::new(hash.as_ref())))
            .await
            .and_then(|r| r.map_status(TORRENT_NOT_FOUND))?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_torrent_web_seeds(
        &self,
        hash: impl AsRef<str> + Send + Sync,
    ) -> Result<Vec<WebSeed>> {
        self.get("torrents/webseeds", Some(&HashArg::new(hash.as_ref())))
            .await
            .and_then(|r| r.map_status(TORRENT_NOT_FOUND))?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_torrent_contents(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        indexes: impl Into<Option<Sep<String, '|'>>> + Send + Sync,
    ) -> Result<Vec<TorrentContent>> {
        #[derive(Serialize)]
        struct Arg<'a> {
            hash: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            indexes: Option<String>,
        }

        self.get(
            "torrents/files",
            Some(&Arg {
                hash: hash.as_ref(),
                indexes: indexes.into().map(|s| s.to_string()),
            }),
        )
        .await
        .and_then(|r| r.map_status(TORRENT_NOT_FOUND))?
        .body_json()
        .await
        .map_err(Into::into)
    }

    pub async fn get_torrent_pieces_states(
        &self,
        hash: impl AsRef<str> + Send + Sync,
    ) -> Result<Vec<PieceState>> {
        self.get("torrents/pieceStates", Some(&HashArg::new(hash.as_ref())))
            .await
            .and_then(|r| r.map_status(TORRENT_NOT_FOUND))?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn get_torrent_pieces_hashes(
        &self,
        hash: impl AsRef<str> + Send + Sync,
    ) -> Result<Vec<String>> {
        self.get("torrents/pieceHashes", Some(&HashArg::new(hash.as_ref())))
            .await
            .and_then(|r| r.map_status(TORRENT_NOT_FOUND))?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn pause_torrents(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        self.get("torrents/pause", Some(&HashesArg::new(hashes)))
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn resume_torrents(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        self.get("torrents/resume", Some(&HashesArg::new(hashes)))
            .await?
            .body_json()
            .await
            .map_err(Into::into)
    }

    pub async fn delete_torrents(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        delete_files: impl Into<Option<bool>> + Send + Sync,
    ) -> Result<()> {
        #[derive(Serialize)]
        #[skip_serializing_none]
        struct Arg {
            hashes: Hashes,
            delete_files: Option<bool>,
        }
        self.get(
            "torrents/delete",
            Some(&Arg {
                hashes: hashes.into(),
                delete_files: delete_files.into(),
            }),
        )
        .await?
        .body_json()
        .await
        .map_err(Into::into)
    }

    pub async fn recheck_torrents(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn reannounce_torrents(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn add_torrent(
        &self,
        src: TorrentSource,
        arg: AddTorrentArg,
    ) -> Result<Vec<Torrent>> {
        todo!()
    }

    pub async fn add_trackers(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        urls: impl Into<Sep<String, '\n'>> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn edit_trackers(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        orig_url: Url,
        new_url: Url,
    ) -> Result<()> {
        todo!()
    }

    pub async fn remove_trackers(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        url: impl AsRef<str> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn add_peers(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        peers: impl Into<Sep<String, '|'>> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn increase_priority(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn decrease_priority(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn maximal_priority(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn minimal_priority(&self, hashes: impl Into<Hashes> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn set_file_priority(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        indexes: impl Into<Sep<i64, '|'>> + Send + Sync,
        priority: Priority,
    ) -> Result<()> {
        todo!()
    }

    pub async fn get_torrent_download_limit(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
    ) -> Result<HashMap<String, u64>> {
        todo!()
    }

    pub async fn set_torrent_download_limit(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        limit: u64,
    ) -> Result<()> {
        todo!()
    }

    pub async fn set_torrent_shared_limit(&self, arg: SetTorrentSharedLimitArg) -> Result<()> {
        todo!()
    }

    pub async fn get_torrent_upload_limit(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
    ) -> Result<HashMap<String, u64>> {
        todo!()
    }

    pub async fn set_torrent_upload_limit(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        limit: u64,
    ) -> Result<()> {
        todo!()
    }

    pub async fn set_torrent_location(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        location: impl AsRef<str> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn set_torrent_name(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        name: impl AsRef<str> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn set_torrent_category(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        category: impl AsRef<str> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn get_categories(&self) -> Result<HashMap<String, Category>> {
        todo!()
    }

    pub async fn add_category(
        &self,
        category: impl AsRef<str> + Send + Sync,
        save_path: impl AsRef<Path> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn edit_category(
        &self,
        category: impl AsRef<str> + Send + Sync,
        save_path: impl AsRef<Path> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn remove_categories(
        &self,
        categories: impl Into<Sep<String, '\n'>> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn add_torrent_tags(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        tags: impl Into<Sep<String, '\n'>> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn remove_torrent_tags(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        tags: Option<impl Into<Sep<String, '\n'>> + Send>,
    ) -> Result<()> {
        todo!()
    }

    pub async fn get_all_tags(&self) -> Result<Vec<String>> {
        todo!()
    }

    pub async fn create_tags(&self, tags: impl Into<Sep<String, ','>> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn delete_tags(&self, tags: impl Into<Sep<String, ','>> + Send + Sync) -> Result<()> {
        todo!()
    }

    pub async fn set_auto_management(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        enable: bool,
    ) -> Result<()> {
        todo!()
    }

    pub async fn toggle_torrent_sequential_download(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn toggle_first_last_piece_priority(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn set_force_start(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        value: bool,
    ) -> Result<()> {
        todo!()
    }

    pub async fn set_super_seeding(
        &self,
        hashes: impl Into<Hashes> + Send + Sync,
        value: bool,
    ) -> Result<()> {
        todo!()
    }

    pub async fn rename_file(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        old_path: impl AsRef<Path> + Send + Sync,
        new_path: impl AsRef<Path> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    pub async fn rename_folder(
        &self,
        hash: impl AsRef<str> + Send + Sync,
        old_path: impl AsRef<Path> + Send + Sync,
        new_path: impl AsRef<Path> + Send + Sync,
    ) -> Result<()> {
        todo!()
    }

    fn url(&self, path: &'static str) -> Url {
        self.endpoint
            .join("api/v2/")
            .unwrap()
            .join(path)
            .expect("Invalid API endpoint")
    }

    async fn login(&self) -> Result<()> {
        if self.cookie.get().is_none() {
            debug!("Cookie not found, logging in");
            let mut req = Request::get(self.url("auth/login"));
            req.set_query(&self.credential)?;
            let Cookie(cookie) = self
                .client
                .send(req)
                .await?
                .map_status(|code| match code as _ {
                    StatusCode::Forbidden => Some(Error::ApiError(ApiError::IpBanned)),
                    _ => None,
                })?
                .extract::<Cookie>()?;

            // Ignore result
            drop(self.cookie.set(cookie));

            debug!("Log in success");
        } else {
            trace!("Already logged in, skipping");
        }

        Ok(())
    }

    async fn request(
        &self,
        method: Method,
        path: &'static str,
        qs: Option<&(impl Serialize + Sync)>,
        body: Option<&(impl Serialize + Sync)>,
    ) -> Result<Response> {
        self.login().await?;
        let mut req = Request::new(method, self.url(path));

        req.append_header(
            headers::COOKIE,
            self.cookie.get().expect("Cookie should be set after login"),
        );

        if let Some(qs) = qs {
            req.set_query(qs)?;
        }

        if let Some(body) = body {
            req.set_body(Body::from_json(body)?);
        }

        trace!(request = ?req, "Sending request");

        self.client
            .send(req)
            .await?
            .map_status(|code| match code as _ {
                StatusCode::Forbidden => Some(Error::ApiError(ApiError::NotLoggedIn)),
                _ => None,
            })
            .tap_ok(|res| trace!(?res))
    }

    // pub async fn add_torrent(&self, urls: )
    async fn get(
        &self,
        path: &'static str,
        qs: Option<&(impl Serialize + Sync)>,
    ) -> Result<Response> {
        self.request(Method::Get, path, qs, Option::<&()>::None)
            .await
    }

    async fn post(
        &self,
        path: &'static str,
        qs: Option<&(impl Serialize + Sync)>,
        body: Option<&(impl Serialize + Sync)>,
    ) -> Result<Response> {
        self.request(Method::Post, path, qs, body).await
    }
}

const NONE: Option<&'static ()> = Option::None;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Http error: {0}")]
    HttpError(http_client::Error),

    #[error("API Returned bad response: {explain}")]
    BadResponse { explain: &'static str },

    #[error("API returned unknown status code: {0}")]
    UnknownHttpCode(StatusCode),

    #[error(transparent)]
    ApiError(#[from] ApiError),
}

/// Errors defined and returned by the API with status code
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("User's IP is banned for too many failed login attempts")]
    IpBanned,

    #[error("API routes requires login, try again")]
    NotLoggedIn,

    #[error("Torrent not found")]
    TorrentNotFound,
}

impl From<http_client::Error> for Error {
    fn from(err: http_client::Error) -> Self {
        Self::HttpError(err)
    }
}

type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod test {
    use std::{env, sync::LazyLock};

    use http_client::h1::H1Client;
    use tracing::info;

    use super::*;

    async fn prepare<'a>() -> Result<&'a Api<H1Client>> {
        static PREPARE: LazyLock<(Credential, Url)> = LazyLock::new(|| {
            dotenv::dotenv().expect("Failed to load .env file");
            tracing_subscriber::fmt::init();

            (
                Credential {
                    username: env::var("QBIT_USERNAME").expect("QBIT_USERNAME not set"),
                    password: env::var("QBIT_PASSWORD").expect("QBIT_PASSWORD not set"),
                },
                env::var("QBIT_BASEURL")
                    .expect("QBIT_BASEURL not set")
                    .parse()
                    .expect("QBIT_BASEURL is not a valid url"),
            )
        });
        static API: OnceLock<Api<H1Client>> = OnceLock::new();

        if let Some(api) = API.get() {
            Ok(api)
        } else {
            let (credential, url) = &*PREPARE;
            let api = Api::new(url.to_owned(), credential.clone(), H1Client::new());
            api.login().await?;
            drop(API.set(api));
            Ok(API.get().unwrap())
        }
    }

    #[tokio::test]
    async fn test_login() {
        let client = prepare().await.unwrap();

        info!(
            version = client.get_version().await.unwrap(),
            "Login success"
        );
    }

    #[tokio::test]
    async fn test_a() {
        let client = prepare().await.unwrap();

        client
            .set_preferences(&Preferences::default())
            .await
            .unwrap();
    }
}
