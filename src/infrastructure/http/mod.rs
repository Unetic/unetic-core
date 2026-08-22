use crate::domain::{CloudflareConfig, DuckDnsConfig};

pub async fn update_cloudflare(cfg: &CloudflareConfig, ip: &str) -> Result<(), String> {
    let url = format!("https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}", cfg.zone_id, cfg.record_id);
    let body = serde_json::json!({ "type": "A", "name": cfg.hostname, "content": ip, "ttl": 60 });
    let client = reqwest::Client::new();
    let resp = client.patch(&url)
        .bearer_auth(&cfg.api_token)
        .json(&body)
        .send().await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() { Ok(()) } else { Err(format!("HTTP {}", resp.status())) }
}

pub async fn update_duckdns(cfg: &DuckDnsConfig, ip: &str) -> Result<(), String> {
    let url = format!("https://www.duckdns.org/update?domains={}&token={}&ip={}", cfg.domain, cfg.token, ip);
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if body.starts_with("OK") { Ok(()) } else { Err(format!("DuckDNS replied: {body}")) }
}
