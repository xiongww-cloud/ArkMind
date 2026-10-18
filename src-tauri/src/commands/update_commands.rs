//! 自动检查更新：查公开分发仓库（xiongww-cloud/Arkmind-Dist）的最新已发布版本。
//! 双端点顺序 failover（国内网络容错）：
//!   ① GitHub API releases/latest（权威源；404 = 尚无已发布 release → 无更新）
//!   ② jsDelivr CDN 读 Dist 仓库 main 分支的 latest.json（CI 在 release publish 时写入并 purge 缓存）
//! 安全边界：镜像 JSON 只携带版本号，下载 URL 由客户端硬编码仓库 slug 构造——
//! 即便 CDN 被投毒，最坏结果是伪造版本号、用户点下载落到真实 GitHub 页，无实质危害。

use serde::Serialize;

/// 分发仓库（公开、仅 Releases、无源码）；下载页 URL 由此硬编码构造
const DIST_REPO_API_LATEST: &str = "https://api.github.com/repos/xiongww-cloud/Arkmind-Dist/releases/latest";
const DIST_REPO_JSDELIVR_LATEST: &str = "https://cdn.jsdelivr.net/gh/xiongww-cloud/Arkmind-Dist@main/latest.json";
const DIST_RELEASES_PAGE: &str = "https://github.com/xiongww-cloud/Arkmind-Dist/releases/tag";
const HTTP_TIMEOUT_SECS: u64 = 5;

#[derive(Serialize)]
pub struct UpdateCheck {
    pub has_update: bool,
    pub latest: String,
    pub url: String,
}

/// 解析版本号（去 v 前缀；少段补 0；第四段忽略）为 (major, minor, patch)——纯函数供测试
pub(crate) fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// latest 是否比 current 新——纯函数供测试；任一侧解析失败一律 false（宁漏报不误报）
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// 由版本号构造 Dist release 页 URL——纯函数供测试；不接受外部输入的 URL（防镜像投毒重定向）
pub(crate) fn release_url(version: &str) -> String {
    let v = version.trim().trim_start_matches('v');
    format!("{}/v{}", DIST_RELEASES_PAGE, v)
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .user_agent("ArkMind-Updater")
        .build()
        .map_err(|e| format!("HTTP client: {}", e))
}

/// 端点①：GitHub API。Ok(Some(tag))=最新已发布版本；Ok(None)=尚无已发布 release；Err=网络/服务错误（走 fallback）
async fn fetch_latest_via_api(client: &reqwest::Client) -> Result<Option<String>, String> {
    let resp = client
        .get(DIST_REPO_API_LATEST)
        .send()
        .await
        .map_err(|e| format!("api.github.com 不可达: {}", e))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub API {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("GitHub API 响应解析: {}", e))?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).unwrap_or("").to_string();
    if tag.is_empty() {
        return Err("GitHub API 响应缺 tag_name".to_string());
    }
    Ok(Some(tag.trim_start_matches('v').to_string()))
}

/// 端点②：jsDelivr 读 Dist 仓库 main 分支 latest.json（{"version":"x.y.z"}）——仅版本号，不含 URL
async fn fetch_latest_via_jsdelivr(client: &reqwest::Client) -> Result<Option<String>, String> {
    let resp = client
        .get(DIST_REPO_JSDELIVR_LATEST)
        .send()
        .await
        .map_err(|e| format!("cdn.jsdelivr.net 不可达: {}", e))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("jsDelivr {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("latest.json 解析: {}", e))?;
    let version = v.get("version").and_then(|t| t.as_str()).unwrap_or("").to_string();
    if version.is_empty() {
        return Err("latest.json 缺 version 字段".to_string());
    }
    Ok(Some(version.trim_start_matches('v').to_string()))
}

/// 检查更新：①权威源失败才走②；两源全挂返回 Err（前端静默或提示，绝不 panic）
#[tauri::command]
pub async fn update_check() -> Result<UpdateCheck, String> {
    let current = env!("CARGO_PKG_VERSION");
    let client = build_client()?;

    let latest = match fetch_latest_via_api(&client).await {
        Ok(v) => v,
        Err(api_err) => {
            log::warn!("[检查更新] GitHub API 失败，降级 jsDelivr: {}", api_err);
            match fetch_latest_via_jsdelivr(&client).await {
                Ok(v) => v,
                Err(cdn_err) => return Err(format!("{}；{}", api_err, cdn_err)),
            }
        }
    };

    let Some(latest) = latest else {
        return Ok(UpdateCheck { has_update: false, latest: String::new(), url: String::new() });
    };
    Ok(UpdateCheck {
        has_update: is_newer(&latest, current),
        url: release_url(&latest),
        latest, // 必须最后 move（结构体字段按书写顺序求值）
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semver_handles_prefix_and_short_forms() {
        assert_eq!(parse_semver("14.4.4"), Some((14, 4, 4)));
        assert_eq!(parse_semver("v14.4.4"), Some((14, 4, 4)));
        assert_eq!(parse_semver("  v15.0.0 "), Some((15, 0, 0)));
        assert_eq!(parse_semver("14.4"), Some((14, 4, 0)), "少段补 0");
        assert_eq!(parse_semver("14.4.3.1"), Some((14, 4, 3)), "第四段忽略");
        assert_eq!(parse_semver(""), None);
        assert_eq!(parse_semver("abc"), None);
        assert_eq!(parse_semver("a.b.c"), None);
    }

    #[test]
    fn is_newer_orders_numerically_and_fails_closed() {
        assert!(is_newer("14.4.4", "14.4.3"));
        assert!(is_newer("15.0.0", "14.9.9"), "major 优先");
        assert!(is_newer("14.5.0", "14.4.9"), "minor 优先");
        assert!(!is_newer("14.4.3", "14.4.3"), "相等不算新");
        assert!(!is_newer("14.4.2", "14.4.3"));
        assert!(is_newer("v14.4.4", "v14.4.3"), "带 v 前缀同口径");
        assert!(!is_newer("abc", "14.4.3"), "解析失败不提示");
        assert!(!is_newer("14.4.4", "abc"));
    }

    #[test]
    fn release_url_is_client_constructed_from_hardcoded_slug() {
        assert_eq!(release_url("14.4.4"), "https://github.com/xiongww-cloud/Arkmind-Dist/releases/tag/v14.4.4");
        assert_eq!(release_url("v14.4.4"), "https://github.com/xiongww-cloud/Arkmind-Dist/releases/tag/v14.4.4", "重复去 v");
        // 镜像投毒边界：即便版本串被塞入奇怪字符，也只会变成无害的 404 路径，不是任意 URL 重定向
        assert!(release_url("../../evil").contains("xiongww-cloud/Arkmind-Dist/releases/tag/"));
    }
}
