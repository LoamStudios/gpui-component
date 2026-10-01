//! A minimal [`HttpClient`] over the workspace `reqwest`, standing in for
//! Zed's `reqwest_client` crate, which GPUI CE does not publish.

use gpui::App;
use gpui::http_client::HttpResponse;
use http::StatusCode;
use std::sync::Arc;

pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    pub fn user_agent(agent: &str) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder().user_agent(agent).build()?;
        Ok(Self { client })
    }
}

impl gpui::http_client::HttpClient for ReqwestClient {
    fn get(
        &self,
        url: &str,
        follow_redirects: bool,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<HttpResponse>> {
        let client = self.client.clone();
        let url = url.to_string();
        Box::pin(async move {
            let response = client
                .get(url)
                .header("follow-redirects", follow_redirects.to_string())
                .send()
                .await?;
            let status = response.status();
            let body = response.bytes().await?;
            Ok(HttpResponse {
                status: StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK),
                body: body.to_vec(),
            })
        })
    }
}

/// Installs a [`ReqwestClient`] as the application HTTP client, so remote
/// images in the showcase load.
#[allow(dead_code)]
pub fn install_http_client(cx: &mut App) {
    match ReqwestClient::user_agent("gpui-base/examples") {
        Ok(http_client) => cx.set_http_client(Arc::new(http_client)),
        Err(error) => log::warn!("failed to create the HTTP client: {error}"),
    }
}
