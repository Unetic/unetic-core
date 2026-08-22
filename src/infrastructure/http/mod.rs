use std::time::Duration;

use crate::domain::{CloudflareConfig, DuckDnsConfig};

pub async fn update_cloudflare(config: &CloudflareConfig, ip: &str) -> Result<(), String> {
    let url = format!(
        "{}/{}/dns_records/{}",
        crate::domain::CLOUDFLARE_API_URL_PREFIX,
        config.zone_id,
        config.record_id
    );
    let body = serde_json::json!({
        "type": "A",
        "name": config.hostname,
        "content": ip,
        "ttl": crate::domain::CLOUDFLARE_DEFAULT_TTL
    });
    let response = client()?
        .patch(url)
        .bearer_auth(&config.api_token)
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let response: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    if response.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err("Cloudflare rejected the DNS update".to_owned())
    }
}

pub async fn update_duckdns(config: &DuckDnsConfig, ip: &str) -> Result<(), String> {
    let response = client()?
        .get(crate::domain::DUCKDNS_API_URL_PREFIX)
        .query(&[
            ("domains", config.domain.as_str()),
            ("token", config.token.as_str()),
            ("ip", ip),
        ])
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let body = response.text().await.map_err(|error| error.to_string())?;
    if body.trim() == "OK" {
        Ok(())
    } else {
        Err("DuckDNS rejected the DNS update".to_owned())
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(crate::domain::HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())
}
