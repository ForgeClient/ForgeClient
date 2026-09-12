//! Game version updater: makes sure a specific engine build is on disk
//! before a replay is played back.
//!
//! Mirrors the Python client's `fa/check.py` + `fa/game_updater/*`: FA
//! refuses to load a replay whose embedded engine version doesn't match the
//! installed one (`"Ack! Unable to load game replay"`), so before *every*
//! replay launch the reference clients diff the local install against the
//! FAF API's file list for that exact `(featured_mod, version)` and update
//! whatever's stale. There is no binary diffing: just per-file MD5
//! comparison, a content-addressed cache, full-file downloads for anything
//! that doesn't match, a tiny 3-offset hex patch of the version number
//! baked into the executable, and a generated `fa_path.lua` FA's Lua
//! bootstrap reads to find everything. Scope: only the base featured-mod
//! types we ever see in replays (`faf`, `ladder1v1`, `fafbeta`,
//! `fafdevelop`): real total-conversion mods needing the base `faf` files
//! *plus* their own overlay is a documented gap, as are map/sim-mod
//! auto-download and per-file progress reporting (all Qt-signal plumbing in
//! the Python client, no architectural equivalent needed here).

use std::io::{Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ports::{PreparationPhase, PreparationStep};

use crate::infra::vault_install::{
    bounded_body, bounded_body_with_progress, install_archive, validate_url, MAX_DOWNLOAD_BYTES,
};

/// Byte offsets inside `ForgedAlliance.exe` where the 4-byte little-endian
/// engine version number is stored. Mirrors `FAPatcher.version_addresses` in
/// the Python client's `fa/game_updater/patcher.py`: these must match
/// exactly or FA reports/behaves as the wrong version.
const VERSION_ADDRESSES: [u64; 3] = [0xd3d40, 0x47612d, 0x476666];

/// One file from `GET /featuredMods/{mod_id}/files/{version}`. `group` is the
/// subdirectory under the target install root (`bin`, `gamedata`, …).
#[derive(Debug, Clone, Deserialize)]
struct FeaturedModFile {
    group: String,
    name: String,
    md5: String,
    /// The release this particular file belongs to. Only meaningful when the
    /// list was fetched as `latest`: see [`effective_version`].
    #[serde(default, deserialize_with = "lenient_i32")]
    version: Option<i32>,
    #[serde(rename = "cacheableUrl")]
    cacheable_url: String,
    #[serde(rename = "hmacToken")]
    hmac_token: String,
    #[serde(rename = "hmacParameter")]
    hmac_parameter: String,
}

/// The API sends `version` as a JSON string (`"3775"`), but has been observed
/// as a bare number too, and it is absent from some older records. Accept all
/// three rather than failing the whole update on a field that is only ever
/// advisory: [`effective_version`] falls back when it is missing.
fn lenient_i32<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64().and_then(|value| i32::try_from(value).ok()),
        _ => None,
    })
}

/// The version a file list actually resolved to.
///
/// Needed because a live game asks for `latest` and never learns the number
/// from the request itself: but `fa_path.lua` and the executable's baked-in
/// version both need it. Both reference clients derive it the same way: prefer
/// the engine executable's own entry (Python's
/// `patch_fa_executable`/`_resolve_base_version`), because that is the version
/// FA will report; otherwise take the highest version in the list (Java's
/// `maxVersion` in `SimpleHttpFeaturedModUpdaterTask`), which is what the
/// release as a whole is called.
fn effective_version(files: &[FeaturedModFile], exe_name: &str) -> Option<i32> {
    files
        .iter()
        .find(|f| f.group == "bin" && f.name.eq_ignore_ascii_case(exe_name))
        .and_then(|f| f.version)
        .or_else(|| files.iter().filter_map(|f| f.version).max())
}

/// A JSON:API document shaped like `{ data: [...] }`: reused here rather
/// than the fuller `JsonApiDoc`/`JsonApiResource` in `infra/jsonapi.rs` since
/// these responses have no `included`/relationships to resolve, just flat
/// attributes per resource.
#[derive(Debug, Deserialize)]
struct JsonApiList {
    #[serde(default)]
    data: Vec<JsonApiEntry>,
}

#[derive(Debug, Deserialize)]
struct JsonApiEntry {
    id: String,
    #[serde(default)]
    attributes: Value,
}

/// Ensure `target_dir` has the exact file set the FAF API lists for
/// `(featured_mod, version)`, then stamp the engine executable's version and
/// write `fa_path.lua`. Idempotent and cheap to call before every replay,
/// files already matching by MD5 are left untouched (mirrors Python calling
/// `check()` unconditionally before each replay rather than pre-checking
/// whether an update is needed).
// Every parameter is independently required and there's a single call site
// (`infra::replay::play_file`): a params struct wouldn't add clarity here.
#[allow(clippy::too_many_arguments)]
pub async fn ensure_game_version(
    http: &reqwest::Client,
    token: &str,
    api_base: &str,
    cache_dir: &Path,
    target_dir: &Path,
    featured_mod: &str,
    version: i32,
    exe_name: &str,
) -> Result<(), String> {
    install_featured_mod(
        http,
        token,
        api_base,
        cache_dir,
        target_dir,
        featured_mod,
        Some(version),
        exe_name,
        false,
        &|_| {},
    )
    .await?;

    write_fa_path_lua(
        target_dir,
        &retail_install_dir(target_dir),
        featured_mod,
        version,
    )?;
    Ok(())
}

/// The featured mods that *are* a complete game install. Everything else
/// (`nomads`, `coop`, total conversions) is an overlay that only ships its own
/// changed files and silently depends on `faf` for the rest.
///
/// Both reference clients hardcode the same idea with slightly different lists
///: Java's `NAMES_OF_FEATURED_BASE_MODS` omits `ladder1v1`, the Python
/// client's `FilesObtainer` includes it. The Python list is used here because
/// it is the superset: treating `ladder1v1` as a base mod is what actually
/// happens on the server (it is the `faf` files under another name), and
/// treating it as an overlay would install `faf` twice for every ladder game.
const BASE_FEATURED_MODS: [&str; 4] = ["faf", "ladder1v1", "fafbeta", "fafdevelop"];

/// Bring the install up to whatever version the server is currently on, for a
/// live game rather than a replay.
///
/// Two differences from [`ensure_game_version`], both load-bearing:
///
/// - **The version is `latest`, not a number.** A live game has no embedded
///   version to read: the server expects every client to be current, and the
///   only way to learn which release that is, is to ask for `latest` and read
///   the version back out of the file list.
/// - **Non-base mods pull `faf` in first.** `nomads` and friends publish only
///   their own changed files; without the base install underneath, the game
///   launches into a missing-file crash. Java's `GameUpdaterImpl::update`
///   ("the featured-mod-mess") and the Python client's `FilesObtainer` both
///   chain the two updates in exactly this order.
///
/// `progress` is called with a user-facing line and measured file progress.
#[allow(clippy::too_many_arguments)]
pub async fn ensure_latest_game_version(
    http: &reqwest::Client,
    token: &str,
    api_base: &str,
    cache_dir: &Path,
    target_dir: &Path,
    featured_mod: &str,
    exe_name: &str,
    cache_rolling_branches: bool,
    progress: &(dyn Fn(PreparationStep) + Sync),
) -> Result<i32, String> {
    let mut base = None;
    if !BASE_FEATURED_MODS.contains(&featured_mod) {
        base = Some(
            install_featured_mod(
                http,
                token,
                api_base,
                cache_dir,
                target_dir,
                "faf",
                None,
                exe_name,
                cache_rolling_branches,
                progress,
            )
            .await?,
        );
    }

    let installed = install_featured_mod(
        http,
        token,
        api_base,
        cache_dir,
        target_dir,
        featured_mod,
        None,
        exe_name,
        cache_rolling_branches,
        progress,
    )
    .await?;

    // `GameVersion` in `fa_path.lua` is the *engine* version, so it comes from
    // whichever step shipped the executable: the overlay almost never does.
    // (The Java client writes the last step's mod version here instead, which
    // for an overlay is a mod revision like `5`; the Python client writes the
    // engine version, which is what the Lua bootstrap actually means by it.)
    let engine_version = installed
        .engine_version
        .or_else(|| base.as_ref().and_then(|b| b.engine_version))
        .unwrap_or(installed.version);

    write_fa_path_lua(
        target_dir,
        &retail_install_dir(target_dir),
        featured_mod,
        engine_version,
    )?;
    Ok(engine_version)
}

/// What one [`install_featured_mod`] pass put on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InstalledMod {
    /// The release the file list resolved to.
    version: i32,
    /// The engine version stamped into `ForgedAlliance.exe`, if this file list
    /// shipped one. Overlay mods (`nomads`, …) never do.
    engine_version: Option<i32>,
}

type CachedCommitSha = (String, String, std::time::Instant);
type CommitShaCache = std::collections::HashMap<String, CachedCommitSha>;

static GITHUB_SHA_CACHE: std::sync::Mutex<Option<CommitShaCache>> = std::sync::Mutex::new(None);

async fn fetch_github_commit_sha(http: &reqwest::Client, branch: &str) -> Option<(String, String)> {
    if let Ok(guard) = GITHUB_SHA_CACHE.lock() {
        if let Some(map) = guard.as_ref() {
            if let Some((sha, url, exp)) = map.get(branch) {
                if std::time::Instant::now() < *exp {
                    return Some((sha.clone(), url.clone()));
                }
            }
        }
    }

    let url = format!("https://api.github.com/repos/FAForever/fa/commits/{branch}");
    let resp = http
        .get(&url)
        .header(reqwest::header::USER_AGENT, "FAForever-Rust-Client")
        .header(reqwest::header::ACCEPT, "application/vnd.github.v3+json")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let val: serde_json::Value = resp.json().await.ok()?;
    let sha = val.get("sha")?.as_str()?.to_string();
    let html_url = val
        .get("html_url")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("https://github.com/FAForever/fa/commit/{sha}"));

    if let Ok(mut guard) = GITHUB_SHA_CACHE.lock() {
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        map.insert(
            branch.to_string(),
            (
                sha.clone(),
                html_url.clone(),
                std::time::Instant::now() + std::time::Duration::from_secs(600),
            ),
        );
    }

    Some((sha, html_url))
}

/// Sync one featured mod's file set into `target_dir` and stamp the engine
/// executable, returning the version that was actually installed.
///
/// Deliberately does *not* write `fa_path.lua`: the overlay chain above calls
/// this twice, and only the last call's featured mod and version belong in
/// that file (mirrors Java writing it once, after the whole chain, from the
/// final `PatchResult`).
#[allow(clippy::too_many_arguments)]
async fn install_featured_mod(
    http: &reqwest::Client,
    token: &str,
    api_base: &str,
    cache_dir: &Path,
    target_dir: &Path,
    featured_mod: &str,
    version: Option<i32>,
    exe_name: &str,
    cache_rolling_branches: bool,
    progress: &(dyn Fn(PreparationStep) + Sync),
) -> Result<InstalledMod, String> {
    // `Updater.run` sets "Requesting files from API..." before anything else
    // happens, for the same reason it is here: two API round trips on a slow
    // connection is long enough for an unlabelled dialog to read as a hang.
    progress(PreparationStep::indeterminate(
        PreparationPhase::Asking,
        format!("Asking the API which files {featured_mod} is made of…"),
    ));
    let mod_id = fetch_mod_id(http, token, api_base, featured_mod).await?;
    let files = fetch_file_list(http, token, api_base, &mod_id, featured_mod, version).await?;

    // Requested version wins; `latest` resolves from the list itself. Falling
    // back to 0 would silently mis-stamp the executable, so an unresolvable
    // version is an error: it means the API gave us files we can't identify.
    let resolved = match version {
        Some(v) => v,
        None => effective_version(&files, exe_name).ok_or_else(|| {
            format!("the API did not say which version '{featured_mod}' is currently on")
        })?,
    };

    let total = files.len();

    // Two passes, as `UpdaterWorker.update_files` does in the Python client:
    // checksum everything first, then fetch only what the checksums rejected.
    //
    // One pass that hashes-and-fetches per file is fewer lines and was what
    // this did, but it cannot say either of the two things a player waiting on
    // it wants to know. It cannot narrate the checksum pass, because a file
    // that matches is simply skipped without a word -- so an install that is
    // already current, which is nearly every launch, reported *nothing* for
    // however long it took to read a few hundred files. And it cannot say what
    // is about to be downloaded, because it only discovers the next outdated
    // file after finishing the previous one.
    let outdated = checksum_pass(target_dir, &files, featured_mod, resolved, progress).await;

    if outdated.is_empty() {
        // `on_mod_progress` in the Python dialog, for `ProgressInfo(0, 0, "")`.
        progress(PreparationStep::counted(
            PreparationPhase::Downloading,
            format!("{featured_mod} {resolved} is up to date ({total} files)"),
            1,
            1,
        ));
    } else {
        let pending = outdated.len();
        progress(PreparationStep::counted(
            PreparationPhase::Downloading,
            format!("{pending} of {total} files need updating for {featured_mod} {resolved}",),
            0,
            pending,
        ));
        for (done, file) in outdated.iter().enumerate() {
            update_file(
                http,
                api_base,
                cache_dir,
                target_dir,
                file,
                featured_mod,
                resolved,
                done,
                pending,
                progress,
            )
            .await?;
        }
    }

    let shipped_exe = files
        .iter()
        .find(|f| f.group == "bin" && f.name.eq_ignore_ascii_case(exe_name));

    // Stamp the executable when we know what to stamp it with. An explicit
    // version is always stamped, including onto an executable the file list
    // didn't mention: that is how replay playback has always worked, and the
    // version is exact by construction there. Resolving `latest`, though, only
    // stamps when the list actually shipped the executable: an overlay mod's
    // "version" is a mod revision (`5`), and writing that into the engine
    // header would tell FA it is a build from 2007.
    let engine_version = match (version, shipped_exe) {
        (Some(v), _) => {
            let path = shipped_exe
                .map(|f| target_dir.join(&f.group).join(&f.name))
                .unwrap_or_else(|| target_dir.join("bin").join(exe_name));
            patch_exe_version(&path, v)?;
            Some(v)
        }
        (None, Some(file)) => {
            let v = file.version.unwrap_or(resolved);
            patch_exe_version(&target_dir.join(&file.group).join(&file.name), v)?;
            Some(v)
        }
        (None, None) => None,
    };

    // Record build state info for rolling branches and mods
    let (git_sha, git_short_sha, commit_url) = if featured_mod == "fafdevelop" {
        if let Some((sha, url)) = fetch_github_commit_sha(http, "develop").await {
            let short = sha.chars().take(7).collect::<String>();
            (Some(sha), Some(short), Some(url))
        } else {
            (
                None,
                None,
                Some("https://github.com/FAForever/fa/commits/develop".to_string()),
            )
        }
    } else if featured_mod == "fafbeta" {
        if let Some((sha, url)) = fetch_github_commit_sha(http, "deploy/fafbeta").await {
            let short = sha.chars().take(7).collect::<String>();
            (Some(sha), Some(short), Some(url))
        } else {
            (
                None,
                None,
                Some("https://github.com/FAForever/fa/commits/deploy/fafbeta".to_string()),
            )
        }
    } else {
        (None, None, None)
    };

    let mut hasher = md5::Context::new();
    for f in &files {
        hasher.consume(f.name.as_bytes());
        hasher.consume(f.md5.as_bytes());
    }
    let composite_hash = format!("{:x}", hasher.compute());
    let short_hash = composite_hash.chars().take(7).collect::<String>();

    let build_info = serde_json::json!({
        "featuredMod": featured_mod,
        "version": version,
        "resolvedVersion": resolved,
        "signature": short_hash,
        "gitSha": git_sha,
        "gitShortSha": git_short_sha,
        "commitUrl": commit_url,
    });
    let _ = std::fs::write(
        target_dir.join(".faf_build.json"),
        serde_json::to_string(&build_info).unwrap_or_default(),
    );

    let entry_name = if featured_mod == "fafdevelop" {
        if let Some(short) = &git_short_sha {
            format!("FAF Develop ({short})")
        } else {
            format!("FAF Develop ({short_hash})")
        }
    } else if featured_mod == "fafbeta" {
        if let Some(short) = &git_short_sha {
            format!("FAF Beta ({short})")
        } else {
            format!("FAF Beta ({short_hash})")
        }
    } else if featured_mod == "faf" || featured_mod == "ladder1v1" {
        format!("FAF Build {resolved}")
    } else {
        format!("{featured_mod} v{resolved}")
    };

    let entry_url = if featured_mod == "fafdevelop" {
        commit_url
            .clone()
            .or_else(|| Some("https://github.com/FAForever/fa/commits/develop".to_string()))
    } else if featured_mod == "fafbeta" {
        commit_url
            .clone()
            .or_else(|| Some("https://github.com/FAForever/fa/commits/deploy/fafbeta".to_string()))
    } else if (featured_mod == "faf" || featured_mod == "ladder1v1") && resolved >= 3636 {
        Some(format!(
            "https://github.com/FAForever/fa/releases/tag/{resolved}"
        ))
    } else {
        None
    };

    let cached_files: Vec<CachedFileInfo> = files
        .iter()
        .map(|f| CachedFileInfo {
            group: f.group.clone(),
            md5: f.md5.clone(),
            name: Some(f.name.clone()),
        })
        .collect();

    let manifest_entry = CacheManifestEntry {
        featured_mod: featured_mod.to_string(),
        version,
        resolved_version: resolved,
        name: entry_name,
        url: entry_url,
        git_short_sha,
        signature: Some(short_hash),
        files: cached_files,
        updated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };

    let is_rolling = featured_mod == "fafdevelop" || featured_mod == "fafbeta";
    if !is_rolling || cache_rolling_branches {
        save_cache_manifest_entry(cache_dir, manifest_entry);
    }

    Ok(InstalledMod {
        version: resolved,
        engine_version,
    })
}

/// The retail Supreme Commander: Forged Alliance install root: where the
/// base-game `movies`, `sounds`, `fonts`, and `gamedata/*.scd` live. This is
/// **not** the FAF patch dir (`target_dir`, e.g. `.../replaydata`), which
/// only holds FAF's `.nx2` gamedata overrides and the patched executable.
///
/// Mirrors the Python client's `ForgedAlliance/app/path` setting, which
/// `writeFAPathLua` writes verbatim as `fa_path`. Getting this wrong is
/// invisible-but-crippling: the FAF init script mounts `fa_path/movies`,
/// `fa_path/sounds`, `fa_path/fonts`: point `fa_path` at the FAF patch dir
/// (which has none of those) and the game still *runs* (base unit/effect
/// blueprints come from the `.nx2` files, mounted relative to the exe), but
/// with no loading-screen movie, no audio, and broken menu fonts.
/// Confirmed live as the cause of exactly that symptom.
///
/// Resolution order: explicit `FAF_GAME_INSTALL_DIR` override → the path the
/// Java or Python client already has configured
/// ([`crate::infra::game::reference_retail_install_paths`]) → auto-detect
/// among the usual retail/Steam locations → `target_dir` as a last resort
/// (preserves the old behaviour rather than writing a knowingly bogus path
/// when nothing is found). Every candidate but the explicit override is
/// validated by `gamedata/lua.scd`, the same probe file Python's
/// `validate_game_path` uses.
///
/// The reference-client configs come before the guessed locations because
/// guessing only ever covers installs under `%ProgramFiles%`: a retail install
/// at, say, `C:\Games\THQ\Gas Powered Games\Supreme Commander - Forged
/// Alliance` fell through to the fallback and produced exactly the broken
/// game described above, silently. Hence the log lines: this decision is
/// otherwise invisible until someone reads a game log.
fn retail_install_dir(target_dir: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("FAF_GAME_INSTALL_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let candidates = crate::infra::game::reference_retail_install_paths()
        .into_iter()
        .chain(typical_retail_install_paths());
    resolve_retail_install_dir(candidates, target_dir)
}

fn resolve_retail_install_dir(
    candidates: impl IntoIterator<Item = PathBuf>,
    target_dir: &Path,
) -> PathBuf {
    if let Some(dir) = candidates.into_iter().find(|p| is_retail_install(p)) {
        tracing::info!(path = %dir.display(), "resolved retail FA install for fa_path");
        return dir;
    }
    tracing::warn!(
        fallback = %target_dir.display(),
        "no retail FA install found: fa_path falls back to the FAF patch dir, so the game \
         will start without base textures, sounds, movies or unit animations. Set \
         FAF_GAME_INSTALL_DIR to the install root holding gamedata/lua.scd."
    );
    target_dir.to_path_buf()
}

/// The probe the reference clients use to tell a real FA root from any other
/// directory: `gamedata/lua.scd` ships only with the base game, never with
/// FAF's `.nx2` overlay.
fn is_retail_install(dir: &Path) -> bool {
    dir.join("gamedata").join("lua.scd").is_file()
}

/// Candidate retail install locations, mirroring the Python client's
/// `typicalForgedAlliancePaths` (THQ/GPG retail, bare retail, and the Steam
/// library: both `%ProgramFiles%` and `%ProgramFiles(x86)%`, since the
/// 32-bit game usually sits under the x86 tree).
fn typical_retail_install_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let program_files_vars = ["ProgramFiles(x86)", "ProgramFiles"];
    let suffixes = [
        r"THQ\Gas Powered Games\Supreme Commander - Forged Alliance",
        r"Supreme Commander - Forged Alliance",
        r"Steam\steamapps\common\Supreme Commander Forged Alliance",
    ];
    for var in program_files_vars {
        if let Ok(base) = std::env::var(var) {
            if base.is_empty() {
                continue;
            }
            for suffix in suffixes {
                out.push(PathBuf::from(&base).join(suffix));
            }
        }
    }
    out
}

async fn fetch_mod_id(
    http: &reqwest::Client,
    token: &str,
    api_base: &str,
    featured_mod: &str,
) -> Result<String, String> {
    let mut url = url::Url::parse(&format!("{api_base}/data/featuredMod"))
        .map_err(|e| format!("invalid API base: {e}"))?;
    url.query_pairs_mut()
        .append_pair("filter", &format!(r#"technicalName=="{featured_mod}""#));

    let resp = http
        .get(url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/vnd.api+json")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "/data/featuredMod returned {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    let doc: JsonApiList = serde_json::from_str(&body).map_err(|e| format!("invalid JSON: {e}"))?;
    doc.data
        .into_iter()
        .next()
        .map(|e| e.id)
        .ok_or_else(|| format!("no featured mod named '{featured_mod}'"))
}

/// Fetch the file list for one release, or for `latest` when `version` is
/// `None`: the exact segment both reference clients use for "whatever the
/// server is on right now" (Java's `getFeaturedModFiles(mod, null)`, the
/// Python client's `_resolve_base_version` returning `"latest"`).
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct CachedFileList {
    expires_at: Instant,
    files: Vec<FeaturedModFile>,
}

type FileListCache = std::collections::HashMap<(String, Option<i32>), CachedFileList>;

static FILE_LIST_CACHE: Mutex<Option<FileListCache>> = Mutex::new(None);

pub fn clear_file_list_cache() {
    if let Ok(mut lock) = FILE_LIST_CACHE.lock() {
        *lock = None;
    }
}

async fn fetch_file_list(
    http: &reqwest::Client,
    token: &str,
    api_base: &str,
    mod_id: &str,
    featured_mod: &str,
    version: Option<i32>,
) -> Result<Vec<FeaturedModFile>, String> {
    // Rolling branches (fafdevelop, fafbeta) change frequently with git commits, so skip file list cache
    let is_rolling = featured_mod == "fafdevelop" || featured_mod == "fafbeta";
    let cache_key = (mod_id.to_string(), version);
    if !is_rolling {
        if let Ok(guard) = FILE_LIST_CACHE.lock() {
            if let Some(cache) = guard.as_ref() {
                if let Some(entry) = cache.get(&cache_key) {
                    if Instant::now() < entry.expires_at {
                        return Ok(entry.files.clone());
                    }
                }
            }
        }
    }

    let version_str = version.map_or_else(|| "latest".to_string(), |v| v.to_string());
    let url = format!("{api_base}/featuredMods/{mod_id}/files/{version_str}");
    let resp = http
        .get(&url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/vnd.api+json")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "{url} returned {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }

    let doc: JsonApiList = serde_json::from_str(&body).map_err(|e| format!("invalid JSON: {e}"))?;
    let files: Vec<FeaturedModFile> = doc
        .data
        .into_iter()
        .map(|e| {
            serde_json::from_value(e.attributes)
                .map_err(|err| format!("invalid featuredModFile attributes: {err}"))
        })
        .collect::<Result<_, _>>()?;

    if !is_rolling {
        if let Ok(mut guard) = FILE_LIST_CACHE.lock() {
            let cache = guard.get_or_insert_with(std::collections::HashMap::new);
            cache.insert(
                cache_key,
                CachedFileList {
                    expires_at: Instant::now() + Duration::from_secs(600), // 10 minutes TTL
                    files: files.clone(),
                },
            );
        }
    }

    Ok(files)
}

/// Read every listed file and compare it against the API's MD5, naming each
/// one as it goes; answer with the ones that need fetching.
///
/// `_calculate_md5s` in the Python client's `UpdaterWorker`, which emits
/// `hash_progress` per file and drives its own bar in the dialog. The point of
/// narrating a pass that usually changes nothing is that it is the pass that
/// takes the time: a few hundred files read off disk, every launch, whether or
/// not a single byte turns out to be stale.
///
/// A file whose checksum cannot be read at all -- missing, unreadable, the
/// wrong length -- counts as outdated and is left to [`update_file`], which is
/// where a bad checksum from the API becomes an error.
async fn checksum_pass<'a>(
    target_dir: &Path,
    files: &'a [FeaturedModFile],
    featured_mod: &str,
    resolved: i32,
    progress: &(dyn Fn(PreparationStep) + Sync),
) -> Vec<&'a FeaturedModFile> {
    let total = files.len();
    let mut outdated = Vec::new();
    for (index, file) in files.iter().enumerate() {
        progress(PreparationStep::counted(
            PreparationPhase::Verifying,
            format!(
                "Checking {featured_mod} {resolved}: {} ({}/{total})",
                file.name,
                index + 1
            ),
            index + 1,
            total,
        ));
        if !file_matches_checksum(target_dir, file).await {
            outdated.push(file);
        }
    }
    outdated
}

/// Whether the file on disk already is the one the API listed.
async fn file_matches_checksum(target_dir: &Path, file: &FeaturedModFile) -> bool {
    let Ok(target_path) = safe_join_file(target_dir, &file.group, &file.name) else {
        return false;
    };
    let Ok(bytes) = tokio::fs::read(&target_path).await else {
        return false;
    };
    format!("{:x}", md5::compute(&bytes)).eq_ignore_ascii_case(&file.md5)
}

/// Whether a download URL the API handed us may be requested, and handed the
/// HMAC token that authorises it.
///
/// The file list arrives from the API with a `cacheable_url` per file, and it
/// was fetched as given: the only integrity anchor was the MD5 in the same
/// document, and MD5 collides. A compromised or mis-served API could point the
/// download anywhere and be sent the token with it.
///
/// The rule is the one `vault_install::validate_url` uses, loosened only where
/// FAF really does spread files across hosts: same site as the API base, which
/// covers `content.faforever.com` and any `*.faforever.com` cache, and nothing
/// else. A host with no dot in it (a local test server) has to match exactly.
fn is_allowed_download_host(raw: &str, api_base: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let Ok(base) = url::Url::parse(api_base) else {
        return false;
    };
    // Never plaintext unless the configured base itself is, which is only ever
    // a deliberate local setup.
    if url.scheme() != "https" && url.scheme() != base.scheme() {
        return false;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let (Some(host), Some(base_host)) = (url.host_str(), base.host_str()) else {
        return false;
    };
    if host.eq_ignore_ascii_case(base_host) {
        return true;
    }
    // The API base's registrable site: the last two labels of its host.
    let mut labels = base_host.rsplitn(3, '.');
    let (Some(tld), Some(domain)) = (labels.next(), labels.next()) else {
        return false;
    };
    let site = format!("{domain}.{tld}");
    host.eq_ignore_ascii_case(&site)
        || host
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", site.to_ascii_lowercase()))
}

/// Bring one outdated file up to date: serve from the content-addressed cache
/// or download fresh (populating the cache either way, for reuse across
/// versions/replays that share a file).
///
/// The caller has already established that this file does not match, so there
/// is no checksum shortcut here: `done`/`total` count the files being
/// *fetched*, not the whole mod.
#[allow(clippy::too_many_arguments)]
async fn update_file(
    http: &reqwest::Client,
    api_base: &str,
    cache_dir: &Path,
    target_dir: &Path,
    file: &FeaturedModFile,
    featured_mod: &str,
    resolved: i32,
    done: usize,
    total: usize,
    progress: &(dyn Fn(PreparationStep) + Sync),
) -> Result<(), String> {
    let target_path = safe_join_file(target_dir, &file.group, &file.name)?;
    if !is_allowed_download_host(&file.cacheable_url, api_base) {
        return Err(format!(
            "the API pointed {} at a download outside FAF",
            file.name
        ));
    }
    if file.md5.len() != 32 || !file.md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "the API returned an invalid checksum for {}",
            file.name
        ));
    }

    let detail = format!(
        "Updating {featured_mod} {resolved}: {} ({}/{total})",
        file.name,
        done + 1
    );
    progress(PreparationStep::counted(
        PreparationPhase::Downloading,
        detail.clone(),
        done,
        total,
    ));

    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }

    let cache_path = safe_join_file(cache_dir, &file.group, &file.md5)?;
    if let Ok(cached) = tokio::fs::read(&cache_path).await {
        if format!("{:x}", md5::compute(&cached)).eq_ignore_ascii_case(&file.md5) {
            tokio::fs::copy(&cache_path, &target_path)
                .await
                .map_err(|e| format!("could not copy cached {}: {e}", file.name))?;
            progress(PreparationStep::counted(
                PreparationPhase::Downloading,
                detail,
                done + 1,
                total,
            ));
            return Ok(());
        }
        // A killed prior write or external cache edit must not be promoted
        // into the live game merely because its filename looks like an MD5.
        let _ = tokio::fs::remove_file(&cache_path).await;
    }

    // The hmac fields are an HTTP header, not a query param, despite the
    // field name: mirrors `BaseDownload.prepare_request` in the Python
    // client's `downloadManager/__init__.py`: `setRawHeader(hmac_parameter,
    // hmac_token)`. A custom User-Agent is set there too; some CDN configs
    // gate on it, so we send the same one.
    let resp = http
        .get(&file.cacheable_url)
        .header(&file.hmac_parameter, &file.hmac_token)
        .header(reqwest::header::USER_AGENT, "FAF Client")
        .send()
        .await
        .map_err(|e| format!("could not download {}: {e}", file.name))?;
    if !resp.status().is_success() {
        return Err(format!(
            "could not download {}: {}",
            file.name,
            resp.status()
        ));
    }
    // Byte progress while the file is in flight, which is the Python dialog's
    // `on_download_progress`: without it a single large file is one unmoving
    // line for however long the CDN takes.
    //
    // Throttled to whole percent. `on_bytes` fires per chunk, and an event per
    // chunk would put thousands of snapshots through the bus to redraw the
    // same bar.
    let last_percent = std::sync::atomic::AtomicU8::new(u8::MAX);
    let bytes = bounded_body_with_progress(
        resp,
        &file.name,
        MAX_DOWNLOAD_BYTES,
        &|received, declared| {
            let Some(size) = declared.filter(|size| *size > 0) else {
                return;
            };
            let percent = ((received.min(size) * 100) / size) as u8;
            if last_percent.swap(percent, std::sync::atomic::Ordering::Relaxed) == percent {
                return;
            }
            progress(PreparationStep::counted(
                PreparationPhase::Downloading,
                format!(
                    "{detail}: {:.1} MB of {:.1} MB",
                    received as f64 / (1024.0 * 1024.0),
                    size as f64 / (1024.0 * 1024.0)
                ),
                done * 100 + percent as usize,
                total * 100,
            ));
        },
    )
    .await?;
    if !format!("{:x}", md5::compute(&bytes)).eq_ignore_ascii_case(&file.md5) {
        return Err(format!("downloaded {} failed its checksum", file.name));
    }

    if let Some(parent) = cache_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    tokio::fs::write(&cache_path, &bytes)
        .await
        .map_err(|e| format!("could not write cache for {}: {e}", file.name))?;
    tokio::fs::write(&target_path, &bytes)
        .await
        .map_err(|e| format!("could not write {}: {e}", target_path.display()))?;
    progress(PreparationStep::counted(
        PreparationPhase::Downloading,
        detail,
        done + 1,
        total,
    ));
    Ok(())
}

fn safe_join_file(root: &Path, group: &str, name: &str) -> Result<PathBuf, String> {
    let safe_relative = |value: &str| {
        if value.contains('\\') || value.contains(':') {
            return false;
        }
        let mut components = Path::new(value).components();
        let has_component = components
            .next()
            .is_some_and(|part| matches!(part, std::path::Component::Normal(_)));
        has_component && components.all(|part| matches!(part, std::path::Component::Normal(_)))
    };
    if !safe_relative(group) || !safe_relative(name) {
        return Err("the API returned a file path outside the game directory".into());
    }
    Ok(root.join(group).join(name))
}

/// Stamp `version` (little-endian, 4 bytes) into the three fixed offsets in
/// the FA executable.
///
/// Every failure here propagates: the callers use `?`. That is deliberate.
/// A half-patched or unpatched executable reports the wrong engine version,
/// which desyncs against everyone else in the lobby, so failing the update is
/// better than launching a subtly wrong game.
///
/// **The size check is load-bearing.** `Seek` past the end of a file is legal,
/// and the following write extends it, filling the gap with zero bytes. Without
/// the check, pointing this at any file smaller than the real executable (a
/// stub, a truncated download, or the `bin/<exe>` fallback below when the file
/// list shipped no executable at all) would not fail: it would silently produce
/// a corrupt multi-megabyte `ForgedAlliance.exe` in the user's install, which
/// only shows up when the game refuses to start.
fn patch_exe_version(exe_path: &Path, version: i32) -> Result<(), String> {
    let required = VERSION_ADDRESSES
        .iter()
        .copied()
        .max()
        .unwrap_or_default()
        .saturating_add(std::mem::size_of::<i32>() as u64);

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(exe_path)
        .map_err(|e| format!("could not open {} for patching: {e}", exe_path.display()))?;

    let length = file
        .metadata()
        .map_err(|e| format!("could not measure {}: {e}", exe_path.display()))?
        .len();
    if length < required {
        return Err(format!(
            "{} is {length} bytes, too small to be Forged Alliance (the version \
             offsets need at least {required}): refusing to patch it",
            exe_path.display()
        ));
    }

    let bytes = version.to_le_bytes();
    for &addr in &VERSION_ADDRESSES {
        file.seek(SeekFrom::Start(addr))
            .map_err(|e| format!("could not seek to {addr:#x}: {e}"))?;
        file.write_all(&bytes)
            .map_err(|e| format!("could not patch version at {addr:#x}: {e}"))?;
    }
    Ok(())
}

/// Mirrors `fa/path.py:writeFAPathLua`. Written into `target_dir` (the FAF
/// patch dir, e.g. `.../replaydata`); the FAF init script beside the exe
/// reads it to locate everything else.
///
/// `fa_path` is the **retail install root** ([`retail_install_dir`]), not
/// `target_dir`: the init script mounts `fa_path/{movies,sounds,fonts}` and
/// `fa_path/gamedata/*.scd` from it, none of which exist under the FAF patch
/// dir. This matches Python writing its `ForgedAlliance/app/path` setting
/// verbatim. (FAF's own `.nx2` gamedata overrides are mounted separately by
/// the init script, relative to the exe: `InitFileDir/../gamedata`: so
/// they keep coming from `target_dir` regardless of `fa_path`.)
///
/// `custom_vault_path` is the user's actual vault root
/// ([`documents_vault_dir`], mirroring Python's `util.VAULTS_BASE_DIR`),
/// the same root [`default_map_search_dirs`]'s first entry stages maps into,
/// so the two never diverge.
fn write_fa_path_lua(
    target_dir: &Path,
    retail_dir: &Path,
    featured_mod: &str,
    version: i32,
) -> Result<(), String> {
    let vault_path = documents_vault_dir().unwrap_or_else(|| target_dir.join("vault"));
    let content = format!(
        "fa_path = \"{}\"\ncustom_vault_path = \"{}\"\nGameType = \"{featured_mod}\"\nGameVersion = \"{version}\"\nClientVersion = \"{}\"\nForceAffinity = false\n",
        slashed(retail_dir),
        slashed(&vault_path),
        env!("CARGO_PKG_VERSION"),
    );
    std::fs::write(target_dir.join("fa_path.lua"), content)
        .map_err(|e| format!("could not write fa_path.lua: {e}"))
}

/// The user's vault root, including a Java-client configured custom vault when
/// available. Shared by
/// [`write_fa_path_lua`] (as `custom_vault_path`) and
/// [`default_map_search_dirs`] (as its first, and primary, map search dir),
/// they must never diverge, since that's exactly the bug this fixes.
fn documents_vault_dir() -> Option<PathBuf> {
    Some(crate::infra::faf_content::vault_dir())
}

fn slashed(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Extract the engine version from a decompressed `.scfareplay` body's
/// leading NUL-terminated string, e.g. `"Supreme Commander v1.50.3684"` →
/// `3684`. Mirrors `ReplayDataParser._game_version` in the Python client's
/// `fa/replayparser.py`.
pub fn extract_game_version(scfa_body: &[u8]) -> Option<i32> {
    let nul = scfa_body.iter().position(|&b| b == 0)?;
    let version_str = std::str::from_utf8(&scfa_body[..nul]).ok()?;
    if !version_str.starts_with("Supreme Commander v1") {
        return None;
    }
    version_str.rsplit('.').next()?.parse().ok()
}

/// The `.scfareplay` header is a sequence of NUL-terminated strings: the
/// SupCom version, a blank "newline" string, then `"{replay_version}\r\n
/// {map_path}"` where `map_path` looks like `/maps/adaptive_gadostb.v0002/
/// adaptive_gadostb.scmap`. Extracts the map's versioned folder name (the
/// second path segment). Mirrors `ReplayDataParser._mapname` in the Python
/// client's `fa/replayparser.py`.
pub fn extract_map_folder(scfa_body: &[u8]) -> Option<String> {
    let mut pos = 0usize;
    read_nul_string(scfa_body, &mut pos)?; // SupCom version string
    read_nul_string(scfa_body, &mut pos)?; // blank "newline" string
    let replay_and_map = read_nul_string(scfa_body, &mut pos)?;
    let map_path = replay_and_map
        .split("\r\n")
        .nth(1)
        .unwrap_or(&replay_and_map);
    let normalized = map_path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if let Some(maps_idx) = parts.iter().position(|p| p.eq_ignore_ascii_case("maps")) {
        if let Some(folder) = parts.get(maps_idx + 1) {
            return Some(folder.to_string());
        }
    }
    if let Some(folder) = parts
        .iter()
        .find(|p| p.to_ascii_lowercase().starts_with("neroxis_map_generator_"))
    {
        return Some(folder.to_string());
    }
    None
}

fn read_nul_string(body: &[u8], pos: &mut usize) -> Option<String> {
    let start = *pos;
    while *pos < body.len() && body[*pos] != 0 {
        *pos += 1;
    }
    if *pos >= body.len() {
        return None;
    }
    let s = String::from_utf8_lossy(&body[start..*pos]).into_owned();
    *pos += 1; // skip the NUL
    Some(s)
}

/// The two directories FA's replay-mode init scripts may search for maps:
/// the user's real vault ([`documents_vault_dir`], honored by
/// `custom_vault_path`-aware init scripts) plus a second, legacy hardcoded
/// fallback under the replay install itself: old replays' init scripts
/// predate the "custom vault path" feature and never consult
/// `fa_path.lua`'s `custom_vault_path` for map lookup at all. Mirrors the FAF
/// Discord-documented workaround of manually copying a map into both.
fn default_map_search_dirs(replay_target_dir: &Path) -> Vec<PathBuf> {
    const SUB: &str = "My Games/Gas Powered Games/Supreme Commander Forged Alliance/maps";
    let mut dirs = Vec::new();
    if let Some(vault_dir) = documents_vault_dir() {
        dirs.push(vault_dir.join("maps"));
    }
    dirs.push(replay_target_dir.join("user").join(SUB));
    dirs
}

/// Every FAF vault map folder is named `{slug}.v{NNNN}` (confirmed against
/// every real vault map this project has seen, e.g. `adaptive_gadostb.v0002`
///: the version suffix is how the vault disambiguates map revisions).
/// Official/base-game maps never carry that suffix (`scmp_002`, `X1MP_002`,
/// …): they ship inside the FA install itself, mounted by `init_<mod>.lua`
/// straight from `fa_path`, entirely independent of the vault/custom-vault
/// mechanism this module stages into. Used to skip the vault CDN lookup
/// entirely for base maps: confirmed live (`X1MP_002`) that hitting the CDN
/// for one is a guaranteed, harmless 404 that otherwise surfaces as a
/// misleading "could not stage map" warning for something that was never
/// broken in the first place.
fn is_vault_map_folder(map_folder: &str) -> bool {
    match map_folder.rsplit_once(".v") {
        Some((_, suffix)) => !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

/// Makes sure `map_folder` (e.g. `adaptive_gadostb.v0002`) is present in
/// every directory FA's replay mode searches: downloading the map's zip
/// from the public vault CDN and extracting it into each if it's missing
/// everywhere. A no-op (not fatal) if the download fails: official/base-game
/// maps (`scmp_XXX`) never need this and simply won't be found remotely,
/// which shouldn't block playback of a replay that doesn't actually need a
/// custom map: recognized up front via [`is_vault_map_folder`] so those
/// never even attempt (and can't fail/warn about) a CDN lookup.
pub async fn ensure_map_available(
    http: &reqwest::Client,
    content_base: &str,
    replay_target_dir: &Path,
    map_folder: &str,
) -> Result<(), String> {
    stage_map(
        http,
        content_base,
        &default_map_search_dirs(replay_target_dir),
        map_folder,
    )
    .await
}

/// Makes sure `map_folder` is in the user's maps folder before a *live* game.
///
/// The live counterpart to [`ensure_map_available`], differing in where the map
/// has to land: a live game reads `custom_vault_path` out of `fa_path.lua` and
/// looks under it, so the two extra legacy locations: which exist purely for
/// old replays' init scripts: are not needed.
///
/// `maps_dir` is where *this client* keeps maps, which is normally the same
/// directory. It can be repointed (`FAF_MAPS_DIR`) while `custom_vault_path`
/// stays derived from the user's Documents folder, so when the two differ the
/// map is staged into both: one is where FA will look, the other is where the
/// client's own installed-maps list looks, and a map visible in only one of
/// them is exactly the confusing half-state this avoids.
///
/// Unlike the replay path this is a *hard* requirement, because the server has
/// already put the player in a game on this map: launching without it is a
/// guaranteed failure to load. Base-game maps (`scmp_009`) are not vault maps
/// and return `Ok` untouched.
pub async fn ensure_live_map(
    http: &reqwest::Client,
    content_base: &str,
    maps_dir: &Path,
    map_folder: &str,
    progress: &(dyn Fn(PreparationStep) + Sync),
) -> Result<(), String> {
    let dirs = live_map_dirs(
        maps_dir,
        documents_vault_dir().map(|dir| dir.join("maps")),
        map_folder,
    );
    if dirs.is_empty() {
        return Ok(());
    }
    progress(PreparationStep::indeterminate(
        PreparationPhase::Map,
        format!("Downloading map {map_folder}…"),
    ));
    stage_map(http, content_base, &dirs, map_folder).await
}

/// The live destinations still missing `map_folder`.
///
/// Filtering here rather than letting [`stage_map`] skip on *any* directory
/// having the map: with two destinations, "present in one" is the half-state
/// [`ensure_live_map`] exists to avoid, not a reason to stop.
fn live_map_dirs(maps_dir: &Path, vault_maps: Option<PathBuf>, map_folder: &str) -> Vec<PathBuf> {
    let mut dirs = vec![maps_dir.to_path_buf()];
    if let Some(vault_maps) = vault_maps {
        if vault_maps != maps_dir {
            dirs.push(vault_maps);
        }
    }
    dirs.retain(|dir| !dir.join(map_folder).is_dir());
    dirs
}

/// Where the vault keeps `map_folder`'s archive.
///
/// Lower-cased, because the vault stores every archive under a lower-case name
/// while folder names carry whatever capitalisation their author used - the
/// co-op missions are `SCCA_Coop_A03.v0023`. The CDN is case-sensitive, so
/// asking for the name as given is a 404 for every map with a capital letter
/// in it, which is most co-op missions and a large share of the vault.
///
/// Checked against the live CDN: `scca_coop_a03.v0023.zip` and
/// `africa.v0005.zip` answer 200, and neither answers to its mixed-case
/// spelling. The extracted folder keeps the archive's own capitalisation;
/// only the request is normalised.
fn vault_map_url(content_base: &str, map_folder: &str) -> String {
    format!(
        "{content_base}/maps/{}.zip",
        map_folder.to_ascii_lowercase()
    )
}

/// Download `map_folder` from the vault CDN and extract it into every
/// directory in `dirs`, unless it is already present in one of them.
async fn stage_map(
    http: &reqwest::Client,
    content_base: &str,
    dirs: &[PathBuf],
    map_folder: &str,
) -> Result<(), String> {
    if !is_vault_map_folder(map_folder) {
        return Ok(()); // base/official map: ships with FA, not the vault
    }

    let search_dirs = dirs.to_vec();
    if search_dirs.iter().any(|dir| dir.join(map_folder).is_dir()) {
        return Ok(()); // already somewhere FA will find it
    }

    let url = vault_map_url(content_base, map_folder);
    validate_url(&url, content_base, "maps")?;
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("could not download map {map_folder}: {e}"))?;
    validate_url(resp.url().as_str(), content_base, "maps")?;
    if !resp.status().is_success() {
        return Err(format!(
            "could not download map {map_folder}: {}",
            resp.status()
        ));
    }
    let bytes = bounded_body(resp, &format!("map {map_folder}"), MAX_DOWNLOAD_BYTES).await?;
    let expected_folder = map_folder.to_string();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        for dir in &search_dirs {
            install_archive(&bytes, dir, Some(&expected_folder), |_| Ok(()))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("map extraction task failed: {e}"))?
}

/// Read the 4-byte engine version stamped into `ForgedAlliance.exe` at `0xd3d40`.
pub fn read_exe_version(exe_path: &Path) -> Option<i32> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(exe_path).ok()?;
    let length = file.metadata().ok()?.len();
    let min_len = VERSION_ADDRESSES[0] + 4;
    if length < min_len {
        return None;
    }
    file.seek(SeekFrom::Start(VERSION_ADDRESSES[0])).ok()?;
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf).ok()?;
    let ver = i32::from_le_bytes(buf);
    if (3000..=10000).contains(&ver) {
        Some(ver)
    } else {
        None
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReplayVersionInfo {
    pub mod_name: String,
    pub game_version: Option<i32>,
    pub git_sha: Option<String>,
    pub git_short_sha: Option<String>,
    pub build_signature: Option<String>,
    pub version_name: Option<String>,
    pub launched_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CacheManifest {
    #[serde(default)]
    entries: Vec<CacheManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheManifestEntry {
    pub featured_mod: String,
    pub version: Option<i32>,
    pub resolved_version: i32,
    pub name: String,
    pub url: Option<String>,
    pub git_short_sha: Option<String>,
    pub signature: Option<String>,
    #[serde(default)]
    pub files: Vec<CachedFileInfo>,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedFileInfo {
    pub group: String,
    pub md5: String,
    #[serde(default)]
    pub name: Option<String>,
}

fn stage_entry_files(
    cache_dir: &Path,
    target_dir: &Path,
    entry: &CacheManifestEntry,
) -> Result<(), String> {
    for f in &entry.files {
        let file_name = match &f.name {
            Some(n) => n.as_str(),
            None => f.md5.as_str(),
        };
        let src = cache_dir.join(&f.group).join(&f.md5);
        if !src.is_file() {
            return Err(format!(
                "cached file {}/{} is missing from cache",
                f.group, file_name
            ));
        }
        let dst = target_dir.join(&f.group).join(file_name);
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if file_name.eq_ignore_ascii_case("ForgedAlliance.exe") {
            std::fs::copy(&src, &dst).map_err(|e| format!("could not copy {file_name}: {e}"))?;
        } else {
            link_or_copy(&src, &dst)
                .map_err(|e| format!("could not copy cached file {file_name}: {e}"))?;
        }
    }
    Ok(())
}

fn stage_cached_version(
    cache_dir: &Path,
    target_dir: &Path,
    entry: &CacheManifestEntry,
    manifest: &CacheManifest,
) -> Result<i32, String> {
    if !BASE_FEATURED_MODS.contains(&entry.featured_mod.as_str()) {
        if let Some(base_entry) = manifest.entries.iter().rfind(|e| e.featured_mod == "faf") {
            let _ = stage_entry_files(cache_dir, target_dir, base_entry);
        }
    }
    stage_entry_files(cache_dir, target_dir, entry)?;

    let exe_path = target_dir.join("bin").join("ForgedAlliance.exe");
    if exe_path.is_file() {
        let _ = patch_exe_version(&exe_path, entry.resolved_version);
    }

    write_fa_path_lua(
        target_dir,
        &retail_install_dir(target_dir),
        &entry.featured_mod,
        entry.resolved_version,
    )?;

    let build_info = serde_json::json!({
        "featuredMod": entry.featured_mod,
        "version": entry.version,
        "resolvedVersion": entry.resolved_version,
        "signature": entry.signature,
        "gitShortSha": entry.git_short_sha,
        "commitUrl": entry.url,
    });
    let _ = std::fs::write(
        target_dir.join(".faf_build.json"),
        serde_json::to_string(&build_info).unwrap_or_default(),
    );

    Ok(entry.resolved_version)
}

#[allow(clippy::too_many_arguments)]
pub async fn resolve_and_stage_replay_version(
    http: &reqwest::Client,
    token: &str,
    api_base: &str,
    cache_dir: &Path,
    target_dir: &Path,
    replay_info: &ReplayVersionInfo,
    exe_name: &str,
) -> Result<Option<String>, String> {
    let mod_name = &replay_info.mod_name;
    let is_rolling = mod_name == "fafdevelop" || mod_name == "fafbeta";
    let manifest = load_cache_manifest(cache_dir);

    if is_rolling {
        let candidates: Vec<&CacheManifestEntry> = manifest
            .entries
            .iter()
            .filter(|e| e.featured_mod == *mod_name)
            .collect();

        let chosen = if !candidates.is_empty() {
            // 1. Exact match by git_sha or git_short_sha or signature
            candidates
                .iter()
                .find(|e| {
                    (replay_info.git_sha.is_some()
                        && e.git_short_sha.is_some()
                        && replay_info
                            .git_sha
                            .as_ref()
                            .unwrap()
                            .starts_with(e.git_short_sha.as_ref().unwrap()))
                        || (replay_info.git_short_sha.is_some()
                            && e.git_short_sha == replay_info.git_short_sha)
                        || (replay_info.build_signature.is_some()
                            && e.signature == replay_info.build_signature)
                })
                .copied()
                // 2. Closest snapshot by timestamp proximity to launched_at
                .or_else(|| {
                    if let Some(launched) = replay_info.launched_at {
                        candidates
                            .iter()
                            .min_by_key(|e| e.updated_at.abs_diff(launched))
                            .copied()
                    } else {
                        None
                    }
                })
                // 3. Fallback to newest cached snapshot
                .or_else(|| candidates.last().copied())
        } else {
            None
        };

        if let Some(entry) = chosen {
            match stage_cached_version(cache_dir, target_dir, entry, &manifest) {
                Ok(_) => {
                    tracing::info!(mod_name, name = %entry.name, "restored replay environment from local cache snapshot");
                    return Ok(None);
                }
                Err(err) => {
                    tracing::warn!(%err, "cached snapshot incomplete, falling back to server latest");
                }
            }
        }

        // Rolling mod has no working cache snapshot: update from server latest
        ensure_latest_game_version(
            http,
            token,
            api_base,
            cache_dir,
            target_dir,
            mod_name,
            exe_name,
            true,
            &|_| {},
        )
        .await?;

        let warning = format!(
            "This replay was played on a rolling development build ({mod_name}) that was not in your local cache. Playback is running with the current development build and may desync if scripts changed."
        );
        return Ok(Some(warning));
    }

    // Fixed / numbered release (e.g. faf build 3839)
    if let Some(version) = replay_info.game_version {
        if let Some(entry) = manifest
            .entries
            .iter()
            .find(|e| e.featured_mod == *mod_name && e.resolved_version == version)
        {
            if stage_cached_version(cache_dir, target_dir, entry, &manifest).is_ok() {
                tracing::info!(
                    mod_name,
                    version,
                    "staged replay environment instantly from local cache"
                );
                return Ok(None);
            }
        }

        // Cache miss: download from server API
        ensure_game_version(
            http, token, api_base, cache_dir, target_dir, mod_name, version, exe_name,
        )
        .await?;
        return Ok(None);
    }

    // Fallback if version was unknown
    ensure_latest_game_version(
        http,
        token,
        api_base,
        cache_dir,
        target_dir,
        mod_name,
        exe_name,
        false,
        &|_| {},
    )
    .await?;
    Ok(None)
}

fn link_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.is_file() || dst.is_symlink() {
        if let (Ok(m_src), Ok(m_dst)) = (src.metadata(), dst.metadata()) {
            if m_src.len() == m_dst.len() {
                return Ok(());
            }
        }
        let _ = std::fs::remove_file(dst);
    }
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(src, dst).is_ok() {
            return Ok(());
        }
    }
    std::fs::copy(src, dst).map(|_| ())
}

fn sync_version_folder(cache_dir: &Path, entry: &CacheManifestEntry) {
    let parent = match cache_dir.parent() {
        Some(p) => p,
        None => return,
    };
    let versions_dir = parent.join("versions");
    let folder_name = crate::infra::sanitize_folder_name(&entry.name);
    let version_dir = versions_dir.join(folder_name);

    for f in &entry.files {
        let file_name = match &f.name {
            Some(n) => n.as_str(),
            None => continue,
        };
        let src = cache_dir.join(&f.group).join(&f.md5);
        if !src.is_file() {
            continue;
        }
        let dst = version_dir.join(&f.group).join(file_name);
        let _ = link_or_copy(&src, &dst);
    }
}

fn load_cache_manifest(cache_dir: &Path) -> CacheManifest {
    let manifest_path = cache_dir.join("cache_manifest.json");
    if let Ok(content) = std::fs::read_to_string(manifest_path) {
        if let Ok(m) = serde_json::from_str::<CacheManifest>(&content) {
            return m;
        }
    }
    CacheManifest::default()
}

fn save_cache_manifest_entry(cache_dir: &Path, entry: CacheManifestEntry) {
    if !cache_dir.is_dir() {
        let _ = std::fs::create_dir_all(cache_dir);
    }
    sync_version_folder(cache_dir, &entry);
    let mut manifest = load_cache_manifest(cache_dir);
    manifest.entries.retain(|e| {
        !(e.featured_mod == entry.featured_mod
            && e.resolved_version == entry.resolved_version
            && e.git_short_sha == entry.git_short_sha
            && e.name == entry.name)
    });
    manifest.entries.push(entry);
    let manifest_path = cache_dir.join("cache_manifest.json");
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = std::fs::write(manifest_path, json);
    }
}

fn prune_cache_manifest(cache_dir: &Path) {
    let mut manifest = load_cache_manifest(cache_dir);
    if manifest.entries.is_empty() {
        return;
    }
    let parent = cache_dir.parent();
    let versions_dir = parent.map(|p| p.join("versions"));

    manifest.entries.retain_mut(|entry| {
        entry
            .files
            .retain(|f| cache_dir.join(&f.group).join(&f.md5).is_file());
        let keep = !entry.files.is_empty();
        if !keep {
            if let Some(v_dir) = &versions_dir {
                let folder_name = crate::infra::sanitize_folder_name(&entry.name);
                let _ = std::fs::remove_dir_all(v_dir.join(folder_name));
            }
        }
        keep
    });
    let manifest_path = cache_dir.join("cache_manifest.json");
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = std::fs::write(manifest_path, json);
    }
}

/// Prune files in `cache_dir` older than `max_age_days`. If `max_age_days == 0`, no files are removed.
pub async fn clean_expired_cache_files(
    cache_dir: &Path,
    max_age_days: u32,
) -> Result<usize, String> {
    if max_age_days == 0 || !cache_dir.is_dir() {
        return Ok(0);
    }
    let max_age = std::time::Duration::from_secs(max_age_days as u64 * 86400);
    let now = std::time::SystemTime::now();
    let mut removed = 0;

    let mut stack = vec![cache_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let metadata = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                let mtime = metadata.modified().unwrap_or(now);
                if let Ok(age) = now.duration_since(mtime) {
                    if age > max_age && tokio::fs::remove_file(&path).await.is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }
    if removed > 0 {
        prune_cache_manifest(cache_dir);
    }
    Ok(removed)
}

/// Remove all files from the game files cache and clear in-memory caches.
pub async fn clear_game_cache(cache_dir: &Path) -> Result<(), String> {
    clear_file_list_cache();
    if let Some(parent) = cache_dir.parent() {
        let versions_dir = parent.join("versions");
        if versions_dir.is_dir() {
            let _ = tokio::fs::remove_dir_all(&versions_dir).await;
        }
    }
    if cache_dir.is_dir() {
        let mut entries = tokio::fs::read_dir(cache_dir)
            .await
            .map_err(|e| format!("could not read cache directory: {e}"))?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if let Ok(file_type) = entry.file_type().await {
                if file_type.is_dir() {
                    let _ = tokio::fs::remove_dir_all(&path).await;
                } else {
                    let _ = tokio::fs::remove_file(&path).await;
                }
            }
        }
    }
    Ok(())
}

fn read_game_type_from_fa_path(dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(dir.join("fa_path.lua")).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("GameType = \"") {
            if let Some(game_type) = rest.strip_suffix('"') {
                return Some(game_type.to_string());
            }
        }
    }
    None
}

struct BuildInfo {
    git_short_sha: Option<String>,
    signature: Option<String>,
    commit_url: Option<String>,
}

fn read_build_info(dir: &Path) -> Option<BuildInfo> {
    let content = std::fs::read_to_string(dir.join(".faf_build.json")).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let git_short_sha = val
        .get("gitShortSha")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let signature = val
        .get("signature")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let commit_url = val
        .get("commitUrl")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    Some(BuildInfo {
        git_short_sha,
        signature,
        commit_url,
    })
}

fn dir_files_stats(dir: &Path) -> (usize, u64) {
    let mut files = 0;
    let mut size = 0;
    for sub in &["gamedata", "bin"] {
        let sub_dir = dir.join(sub);
        if let Ok(entries) = std::fs::read_dir(sub_dir) {
            for entry in entries.flatten() {
                if let Ok(m) = entry.metadata() {
                    if m.is_file() {
                        files += 1;
                        size += m.len();
                    }
                }
            }
        }
    }
    (files, size)
}

/// Calculate cache size, count, and discovered game versions.
pub async fn inspect_game_cache(
    cache_dir: &Path,
    install_dirs: &[PathBuf],
) -> faf_domain::state::GameCacheInfo {
    let mut total_size_bytes = 0u64;
    let mut total_files = 0usize;
    let mut existing_cache_files: std::collections::HashMap<(String, String), u64> =
        std::collections::HashMap::new();
    let mut legacy_exe_versions: std::collections::BTreeMap<i32, (usize, u64)> =
        std::collections::BTreeMap::new();

    if cache_dir.is_dir() {
        let mut stack = vec![cache_dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let metadata = match entry.metadata().await {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if metadata.is_dir() {
                    stack.push(path);
                } else if metadata.is_file() {
                    if path.file_name().and_then(|n| n.to_str()) == Some("cache_manifest.json") {
                        continue;
                    }
                    let len = metadata.len();
                    total_size_bytes += len;
                    total_files += 1;

                    if let (Some(md5), Some(group_dir)) =
                        (path.file_name().and_then(|n| n.to_str()), path.parent())
                    {
                        if let Some(group) = group_dir.file_name().and_then(|n| n.to_str()) {
                            existing_cache_files.insert((group.to_string(), md5.to_string()), len);
                        }
                    }

                    if let Some(v) = read_exe_version(&path) {
                        if v >= 3636 {
                            let entry = legacy_exe_versions.entry(v).or_insert((0, 0));
                            entry.0 += 1;
                            entry.1 += len;
                        }
                    }
                }
            }
        }
    }

    let manifest = load_cache_manifest(cache_dir);
    let mut versions: Vec<faf_domain::state::CachedGameVersion> = Vec::new();
    let mut recorded_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in manifest.entries {
        let mut count = 0usize;
        let mut size = 0u64;
        for f in &entry.files {
            if let Some(&file_len) = existing_cache_files.get(&(f.group.clone(), f.md5.clone())) {
                count += 1;
                size += file_len;
            }
        }
        if count > 0 {
            sync_version_folder(cache_dir, &entry);
            recorded_names.insert(entry.name.clone());
            versions.push(faf_domain::state::CachedGameVersion {
                name: entry.name,
                version: entry.resolved_version,
                file_count: count.min(u32::MAX as usize) as u32,
                size_bytes: size as f64,
                url: entry.url,
            });
        }
    }

    // Auto-discover and populate versions from active install_dirs
    for dir in install_dirs {
        let game_type = read_game_type_from_fa_path(dir);
        let build_info = read_build_info(dir);
        let (f_count, f_size) = dir_files_stats(dir);

        if let Some(gt) = game_type {
            let is_develop = gt == "fafdevelop";
            let is_beta = gt == "fafbeta";
            if is_develop || is_beta {
                let (name, url) = match &build_info {
                    Some(info) if info.git_short_sha.is_some() => (
                        format!(
                            "FAF {} ({})",
                            if is_develop { "Develop" } else { "Beta" },
                            info.git_short_sha.as_ref().unwrap()
                        ),
                        info.commit_url.clone().unwrap_or_else(|| {
                            format!(
                                "https://github.com/FAForever/fa/commits/{}",
                                if is_develop {
                                    "develop"
                                } else {
                                    "deploy/fafbeta"
                                }
                            )
                        }),
                    ),
                    Some(info) if info.signature.is_some() => (
                        format!(
                            "FAF {} ({})",
                            if is_develop { "Develop" } else { "Beta" },
                            info.signature.as_ref().unwrap()
                        ),
                        info.commit_url.clone().unwrap_or_else(|| {
                            format!(
                                "https://github.com/FAForever/fa/commits/{}",
                                if is_develop {
                                    "develop"
                                } else {
                                    "deploy/fafbeta"
                                }
                            )
                        }),
                    ),
                    _ => (
                        format!("FAF {}", if is_develop { "Develop" } else { "Beta" }),
                        format!(
                            "https://github.com/FAForever/fa/commits/{}",
                            if is_develop {
                                "develop"
                            } else {
                                "deploy/fafbeta"
                            }
                        ),
                    ),
                };
                if !recorded_names.contains(&name) {
                    recorded_names.insert(name.clone());
                    versions.push(faf_domain::state::CachedGameVersion {
                        name,
                        version: 0,
                        file_count: f_count.min(u32::MAX as usize) as u32,
                        size_bytes: f_size as f64,
                        url: Some(url),
                    });
                }
            } else if gt == "faf" || gt == "ladder1v1" {
                let exe_path = dir.join("bin").join("ForgedAlliance.exe");
                if let Some(v) = read_exe_version(&exe_path) {
                    if v >= 3636 {
                        let name = format!("FAF Build {v}");
                        if !recorded_names.contains(&name) {
                            recorded_names.insert(name.clone());
                            versions.push(faf_domain::state::CachedGameVersion {
                                name,
                                version: v,
                                file_count: f_count.min(u32::MAX as usize) as u32,
                                size_bytes: f_size as f64,
                                url: Some(format!(
                                    "https://github.com/FAForever/fa/releases/tag/{v}"
                                )),
                            });
                        }
                    }
                }
            }
        }
    }

    // Fallback: add standalone legacy PE binaries discovered in cache_dir
    for (version, (file_count, size_bytes)) in legacy_exe_versions {
        let name = format!("FAF Build {version}");
        if !recorded_names.contains(&name) {
            recorded_names.insert(name.clone());
            versions.push(faf_domain::state::CachedGameVersion {
                name,
                version,
                file_count: file_count.min(u32::MAX as usize) as u32,
                size_bytes: size_bytes as f64,
                url: Some(format!(
                    "https://github.com/FAForever/fa/releases/tag/{version}"
                )),
            });
        }
    }

    versions.sort_by(|a, b| b.version.cmp(&a.version).then_with(|| a.name.cmp(&b.name)));

    faf_domain::state::GameCacheInfo {
        total_size_bytes: total_size_bytes as f64,
        total_files: total_files.min(u32::MAX as usize) as u32,
        versions,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_download_url_has_to_stay_on_faf() {
        use super::is_allowed_download_host as allowed;
        let base = "https://api.faforever.com";

        // The hosts FAF really serves files from.
        assert!(allowed(
            "https://content.faforever.com/faf/updaterNew/x.nx2",
            base
        ));
        assert!(allowed("https://api.faforever.com/x", base));
        assert!(allowed("https://faforever.com/x", base));

        // The URL arrives in the same document as the MD5 that is supposed to
        // vouch for it, and it is sent the HMAC token that authorises the
        // download, so neither is evidence about the other.
        assert!(!allowed("https://faforever.com.evil.example/x", base));
        assert!(!allowed("https://evil.example/faforever.com/x", base));
        assert!(
            !allowed("http://content.faforever.com/x", base),
            "no plaintext"
        );
        assert!(
            !allowed("https://user:pw@content.faforever.com/x", base),
            "no credentials in the URL"
        );
        assert!(!allowed("file:///C:/Windows/System32/cmd.exe", base));
        assert!(!allowed("not a url", base));
    }

    #[test]
    fn a_local_test_api_still_works() {
        use super::is_allowed_download_host as allowed;
        // A deliberate local setup has no registrable domain to match on, so
        // the host has to be the same one, and its scheme is allowed to be the
        // base's own.
        let base = "http://localhost:8080";
        assert!(allowed("http://localhost:8080/files/x", base));
        assert!(!allowed("http://elsewhere:8080/files/x", base));
    }

    use super::*;
    use serde_json::json;
    use std::io::Read as _;

    #[test]
    fn a_vault_map_is_requested_under_its_lower_case_name() {
        // The reported failure: hosting never got its map. The vault serves
        // `scca_coop_a03.v0023.zip`, the co-op API names the folder
        // `SCCA_Coop_A03.v0023`, and the CDN is case-sensitive - so the
        // request 404'd for every map with a capital letter in its name.
        assert_eq!(
            vault_map_url("https://content.faforever.com", "SCCA_Coop_A03.v0023"),
            "https://content.faforever.com/maps/scca_coop_a03.v0023.zip"
        );
        // A name that is already lower case is untouched.
        assert_eq!(
            vault_map_url("https://content.faforever.com", "adaptive_gadostb.v0002"),
            "https://content.faforever.com/maps/adaptive_gadostb.v0002.zip"
        );
    }

    #[test]
    fn extracts_game_version_from_supcom_header_string() {
        let mut body = b"Supreme Commander v1.50.3684".to_vec();
        body.push(0);
        body.extend_from_slice(b"rest of the replay");
        assert_eq!(extract_game_version(&body), Some(3684));
    }

    #[test]
    fn extract_game_version_rejects_non_supcom_strings() {
        let mut body = b"not a replay header".to_vec();
        body.push(0);
        assert_eq!(extract_game_version(&body), None);
    }

    #[test]
    fn extract_game_version_none_without_a_nul_terminator() {
        assert_eq!(extract_game_version(b"Supreme Commander v1.50.3684"), None);
    }

    fn scfa_header(version: &str, map_path: &str, trailing: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(version.as_bytes());
        body.push(0);
        body.push(0); // blank "newline" string
        body.extend_from_slice(format!("Replay v1.9\r\n{map_path}").as_bytes());
        body.push(0);
        body.extend_from_slice(trailing);
        body
    }

    #[test]
    fn extracts_map_folder_from_scfa_header() {
        let body = scfa_header(
            "Supreme Commander v1.50.3684",
            "/maps/adaptive_gadostb.v0002/adaptive_gadostb.scmap",
            b"garbage\0rest",
        );
        assert_eq!(
            extract_map_folder(&body).as_deref(),
            Some("adaptive_gadostb.v0002")
        );
    }

    #[test]
    fn extract_map_folder_none_for_non_maps_path() {
        let body = scfa_header("Supreme Commander v1.50.3684", "/not-maps/foo/bar", b"\0");
        assert_eq!(extract_map_folder(&body), None);
    }

    #[test]
    fn default_map_search_dirs_includes_replay_target_user_dir() {
        let target_dir = Path::new(r"C:\ProgramData\FAForever\replaydata");
        let dirs = default_map_search_dirs(target_dir);
        let expected_suffix =
            "user/My Games/Gas Powered Games/Supreme Commander Forged Alliance/maps"
                .replace('/', std::path::MAIN_SEPARATOR_STR);
        assert!(
            dirs.iter().any(|d| d.ends_with(&expected_suffix)),
            "{dirs:?} should include the replay-target user dir"
        );
    }

    #[test]
    fn patches_all_three_version_offsets_little_endian() {
        let dir = std::env::temp_dir().join(format!("forge-patch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake.exe");
        // Large enough to cover the highest offset (0x47612d) plus 4 bytes.
        std::fs::write(&path, vec![0u8; 0x476670]).unwrap();

        patch_exe_version(&path, 3828).expect("should patch");

        let mut file = std::fs::File::open(&path).unwrap();
        for &addr in &VERSION_ADDRESSES {
            let mut buf = [0u8; 4];
            file.seek(SeekFrom::Start(addr)).unwrap();
            file.read_exact(&mut buf).unwrap();
            assert_eq!(i32::from_le_bytes(buf), 3828, "offset {addr:#x}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_patch_a_file_too_small_to_be_the_executable() {
        // The regression this guards: seeking past the end of a short file and
        // writing is legal, and would leave a zero-filled 4.6 MB "executable"
        // behind rather than reporting a problem.
        let dir = std::env::temp_dir().join(format!("forge-patch-small-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stub.exe");
        std::fs::write(&path, b"not really an executable").unwrap();

        let error = patch_exe_version(&path, 3828).expect_err("a stub must not be patched");
        assert!(error.contains("too small"), "{error}");
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            24,
            "the file must be left exactly as it was"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_retail_root_outside_the_guessed_locations_still_wins_over_the_fallback() {
        // The shape of the install this fixes: the right THQ layout, but
        // under `C:\Games`, so `typical_retail_install_paths` never sees it.
        // Before the reference configs were consulted this fell straight
        // through to `target_dir` and the game launched with no base content.
        let temp = tempfile::tempdir().unwrap();
        let retail = temp
            .path()
            .join("Games/THQ/Supreme Commander - Forged Alliance");
        std::fs::create_dir_all(retail.join("gamedata")).unwrap();
        std::fs::write(retail.join("gamedata").join("lua.scd"), b"base game").unwrap();

        let patch_dir = temp.path().join("FAForever/replaydata");
        let guessed_but_absent = temp.path().join("Program Files/THQ/SCFA");

        assert_eq!(
            resolve_retail_install_dir([guessed_but_absent, retail.clone()], &patch_dir),
            retail,
            "the first candidate that actually holds gamedata/lua.scd must win"
        );
    }

    #[test]
    fn the_faf_patch_dir_is_only_a_last_resort() {
        // It has `gamedata/*.nx2` but no `.scd`, so it must never satisfy the
        // probe: reaching it means we knowingly write a degraded `fa_path`,
        // which is what the warning in `resolve_retail_install_dir` is for.
        let temp = tempfile::tempdir().unwrap();
        let patch_dir = temp.path().join("replaydata");
        std::fs::create_dir_all(patch_dir.join("gamedata")).unwrap();
        std::fs::write(patch_dir.join("gamedata").join("units.nx2"), b"overlay").unwrap();

        assert!(!is_retail_install(&patch_dir));
        assert_eq!(
            resolve_retail_install_dir([patch_dir.clone()], &patch_dir),
            patch_dir
        );
    }

    #[test]
    fn writes_fa_path_lua_with_expected_fields() {
        let dir = std::env::temp_dir().join(format!("forge-lua-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let retail = dir.join("retail-install");
        write_fa_path_lua(&dir, &retail, "ladder1v1", 3684).expect("should write");
        let content = std::fs::read_to_string(dir.join("fa_path.lua")).unwrap();

        assert!(content.contains("GameType = \"ladder1v1\""));
        assert!(content.contains("GameVersion = \"3684\""));
        assert!(content.contains("ForceAffinity = false"));
        assert!(!content.contains('\\'), "paths must use forward slashes");
        // fa_path must be the retail install root, never the FAF patch dir,
        // the game mounts movies/sounds/fonts from it (the bug this guards).
        assert!(
            content.contains(&format!("fa_path = \"{}\"", slashed(&retail))),
            "fa_path should be the retail install dir: {content}",
        );
        assert!(
            !content.contains(&format!("fa_path = \"{}/bin\"", slashed(&dir))),
            "fa_path must not point at the FAF patch dir's bin: {content}",
        );
        // Only meaningful on a host that can resolve a Documents dir (not
        // every CI runner can): where it does, `write_fa_path_lua` must use
        // it rather than falling back to the never-populated
        // `target_dir/vault` (the bug this test guards).
        if let Some(vault_dir) = documents_vault_dir() {
            assert!(
                content.contains(&format!("custom_vault_path = \"{}\"", slashed(&vault_dir))),
                "custom_vault_path should be the user's real vault root ({}): {content}",
                slashed(&vault_dir),
            );
            assert!(
                !content.contains(&format!(
                    "custom_vault_path = \"{}",
                    slashed(&dir.join("vault"))
                )),
                "custom_vault_path must not point at an unpopulated target_dir/vault: {content}",
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_featured_mod_file_list_response() {
        let doc: JsonApiList = serde_json::from_value(json!({
            "data": [{
                "type": "featuredModFile",
                "id": "1",
                "attributes": {
                    "group": "bin",
                    "name": "ForgedAlliance.exe",
                    "md5": "abc123",
                    "cacheableUrl": "https://content.example.com/bin/ForgedAlliance.exe",
                    "hmacToken": "tok",
                    "hmacParameter": "verify",
                },
            }],
        }))
        .unwrap();

        let files: Vec<FeaturedModFile> = doc
            .data
            .into_iter()
            .map(|e| serde_json::from_value(e.attributes).unwrap())
            .collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].group, "bin");
        assert_eq!(files[0].name, "ForgedAlliance.exe");
        assert_eq!(files[0].md5, "abc123");
        assert_eq!(files[0].hmac_parameter, "verify");
    }

    fn file(group: &str, name: &str, version: Option<i32>) -> FeaturedModFile {
        FeaturedModFile {
            group: group.into(),
            name: name.into(),
            md5: "abc".into(),
            version,
            cacheable_url: "https://example.invalid/f".into(),
            hmac_token: "tok".into(),
            hmac_parameter: "verify".into(),
        }
    }

    /// A file list whose checksums are the real MD5s of what is written to
    /// `dir`, so a `checksum_pass` over it finds everything current.
    fn staged(dir: &std::path::Path, names: &[(&str, &str)]) -> Vec<FeaturedModFile> {
        names
            .iter()
            .map(|(group, name)| {
                let body = format!("contents of {name}");
                let group_dir = dir.join(group);
                std::fs::create_dir_all(&group_dir).unwrap();
                std::fs::write(group_dir.join(name), &body).unwrap();
                FeaturedModFile {
                    group: (*group).into(),
                    name: (*name).into(),
                    md5: format!("{:x}", md5::compute(body.as_bytes())),
                    version: None,
                    cacheable_url: "https://example.invalid/f".into(),
                    hmac_token: "tok".into(),
                    hmac_parameter: "verify".into(),
                }
            })
            .collect()
    }

    #[tokio::test]
    async fn the_checksum_pass_names_every_file_it_reads() {
        // The report: an install that needs nothing says nothing, for as long
        // as it takes to read a few hundred files. The pass that finds nothing
        // to do is exactly the pass that has to narrate itself, because it is
        // the one that takes the time.
        let dir = tempfile::tempdir().unwrap();
        let files = staged(
            dir.path(),
            &[
                ("bin", "ForgedAlliance.exe"),
                ("gamedata", "units.nx2"),
                ("gamedata", "lua.nx2"),
            ],
        );

        let seen = std::sync::Mutex::new(Vec::new());
        let outdated = checksum_pass(dir.path(), &files, "faf", 3836, &|step| {
            seen.lock().unwrap().push(step);
        })
        .await;

        assert!(outdated.is_empty(), "everything on disk is current");
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 3, "one line per file, not one per download");
        assert!(seen
            .iter()
            .all(|step| step.phase == PreparationPhase::Verifying));
        assert!(seen[0].detail.contains("ForgedAlliance.exe"));
        assert_eq!(
            seen[0].detail,
            "Checking faf 3836: ForgedAlliance.exe (1/3)"
        );
        assert_eq!(
            (seen[0].progress, seen[2].progress),
            (Some(33), Some(100)),
            "the bar tracks files read, so it moves while nothing downloads"
        );
    }

    #[tokio::test]
    async fn a_file_that_does_not_match_is_handed_on_to_be_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = staged(dir.path(), &[("bin", "current.dat"), ("bin", "stale.dat")]);
        // The API moved on: the local copy is now the wrong one.
        files[1].md5 = format!("{:x}", md5::compute(b"a newer build"));
        // And one the install has never had at all.
        files.push(FeaturedModFile {
            group: "bin".into(),
            name: "added.dat".into(),
            md5: format!("{:x}", md5::compute(b"brand new")),
            version: None,
            cacheable_url: "https://example.invalid/f".into(),
            hmac_token: "tok".into(),
            hmac_parameter: "verify".into(),
        });

        let outdated = checksum_pass(dir.path(), &files, "faf", 3836, &|_| {}).await;

        let names: Vec<&str> = outdated.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(
            names,
            ["stale.dat", "added.dat"],
            "a missing file is outdated, not an error: update_file fetches it"
        );
    }

    #[test]
    fn a_file_lists_version_is_accepted_as_a_string_a_number_or_not_at_all() {
        let parse =
            |attributes: Value| -> FeaturedModFile { serde_json::from_value(attributes).unwrap() };
        let base = |extra: Value| {
            let mut v = json!({
                "group": "bin",
                "name": "ForgedAlliance.exe",
                "md5": "abc123",
                "cacheableUrl": "https://example.invalid/f",
                "hmacToken": "tok",
                "hmacParameter": "verify",
            });
            if let (Some(map), Some(more)) = (v.as_object_mut(), extra.as_object()) {
                map.extend(more.clone());
            }
            v
        };

        // The API sends it as a string today; a bare number and an absent
        // field both have to stay non-fatal, since the field only feeds
        // version *inference* and every other path supplies the number.
        assert_eq!(parse(base(json!({"version": "3775"}))).version, Some(3775));
        assert_eq!(parse(base(json!({"version": 3775}))).version, Some(3775));
        assert_eq!(parse(base(json!({}))).version, None);
        assert_eq!(parse(base(json!({"version": null}))).version, None);
        assert_eq!(
            parse(base(json!({"version": 4_294_967_296_u64}))).version,
            None
        );
    }

    #[test]
    fn latest_resolves_to_the_engine_executables_own_version() {
        // Not the highest in the list: a release can ship a newer data file
        // than executable, and FA reports the version baked into the exe.
        let files = [
            file("gamedata", "units.nx2", Some(3777)),
            file("bin", "ForgedAlliance.exe", Some(3775)),
        ];
        assert_eq!(
            effective_version(&files, "ForgedAlliance.exe"),
            Some(3775),
            "the executable's entry wins over the list maximum"
        );
    }

    #[test]
    fn latest_falls_back_to_the_highest_version_without_an_executable() {
        // An overlay mod (`nomads`) ships no executable at all.
        let files = [
            file("gamedata", "nomads.nx2", Some(4)),
            file("gamedata", "nomadsinit.nx2", Some(7)),
        ];
        assert_eq!(effective_version(&files, "ForgedAlliance.exe"), Some(7));
    }

    #[test]
    fn an_unversioned_file_list_resolves_to_nothing() {
        // Better than defaulting to 0, which would stamp the executable as a
        // build that never existed.
        let files = [file("bin", "ForgedAlliance.exe", None)];
        assert_eq!(effective_version(&files, "ForgedAlliance.exe"), None);
    }

    #[test]
    fn the_executable_is_matched_case_insensitively() {
        let files = [file("bin", "forgedalliance.exe", Some(3775))];
        assert_eq!(effective_version(&files, "ForgedAlliance.exe"), Some(3775));
    }

    #[test]
    fn overlay_mods_are_distinguished_from_complete_installs() {
        // Only the four base mods are a whole game; everything else needs
        // `faf` underneath it first.
        for base in ["faf", "ladder1v1", "fafbeta", "fafdevelop"] {
            assert!(BASE_FEATURED_MODS.contains(&base), "{base} is a base mod");
        }
        for overlay in ["nomads", "coop", "murderparty"] {
            assert!(
                !BASE_FEATURED_MODS.contains(&overlay),
                "{overlay} must pull faf in first"
            );
        }
    }

    #[test]
    fn a_live_map_already_in_place_needs_no_destinations() {
        let temp = std::env::temp_dir().join(format!("faf-live-dirs-{}", std::process::id()));
        let maps = temp.join("maps");
        std::fs::create_dir_all(maps.join("adaptive_gadostb.v0002")).unwrap();

        // Normal setup: the client's maps folder *is* the vault maps folder,
        // and the map is there: nothing left to do, so no download.
        assert!(live_map_dirs(&maps, Some(maps.clone()), "adaptive_gadostb.v0002").is_empty());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn a_map_present_in_only_one_live_destination_is_still_staged_into_the_other() {
        let temp = std::env::temp_dir().join(format!("faf-live-split-{}", std::process::id()));
        let maps = temp.join("maps");
        let vault = temp.join("vault-maps");
        std::fs::create_dir_all(maps.join("adaptive_gadostb.v0002")).unwrap();
        std::fs::create_dir_all(&vault).unwrap();

        // The client can list the map while the game cannot find it. Staging
        // has to close that gap rather than reporting success.
        assert_eq!(
            live_map_dirs(&maps, Some(vault.clone()), "adaptive_gadostb.v0002"),
            vec![vault],
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn one_destination_is_never_listed_twice() {
        let temp = std::env::temp_dir().join(format!("faf-live-dedup-{}", std::process::id()));
        let maps = temp.join("maps");

        // Extracting the same zip into the same directory twice is harmless
        // but pointless; the usual case is exactly this one directory.
        assert_eq!(
            live_map_dirs(&maps, Some(maps.clone()), "adaptive_gadostb.v0002"),
            vec![maps],
        );
    }

    #[tokio::test]
    async fn a_base_game_map_needs_no_staging_for_a_live_game() {
        // `scmp_009` ships inside the FA install; the vault has never heard of
        // it, so asking would be a guaranteed 404 that fails the launch.
        let result = ensure_live_map(
            &reqwest::Client::new(),
            "http://127.0.0.1:1",
            std::path::Path::new("definitely/not/here"),
            "scmp_009",
            &|_| {},
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// Builds an in-memory zip shaped like a real vault map download,
    /// confirmed against `content.faforever.com/maps/adaptive_gadostb.v0002.zip`:
    /// a single top-level `{map_folder}/` directory containing the map's files.
    fn build_map_zip(map_folder: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        writer
            .start_file(format!("{map_folder}/{map_folder}.scmap"), options)
            .unwrap();
        writer.write_all(b"fake map bytes").unwrap();
        writer.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn vault_install_places_files_under_the_map_folder() {
        let dir = std::env::temp_dir().join(format!("forge-mapzip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let zip_bytes = build_map_zip("adaptive_gadostb.v0002");
        install_archive(&zip_bytes, &dir, Some("adaptive_gadostb.v0002"), |_| Ok(()))
            .expect("should extract");

        let scmap = dir
            .join("adaptive_gadostb.v0002")
            .join("adaptive_gadostb.v0002.scmap");
        assert_eq!(std::fs::read(&scmap).unwrap(), b"fake map bytes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ensure_map_available_skips_download_when_already_staged() {
        let target_dir =
            std::env::temp_dir().join(format!("forge-maptarget-{}", std::process::id()));
        let user_maps = target_dir
            .join("user")
            .join("My Games")
            .join("Gas Powered Games")
            .join("Supreme Commander Forged Alliance")
            .join("maps")
            .join("adaptive_gadostb.v0002");
        tokio::fs::create_dir_all(&user_maps).await.unwrap();

        let http = reqwest::Client::new();
        // An unreachable content_base proves no network call was attempted,
        // this would error out immediately if the "already staged" short
        // circuit didn't fire.
        ensure_map_available(
            &http,
            "http://127.0.0.1:1",
            &target_dir,
            "adaptive_gadostb.v0002",
        )
        .await
        .expect("should skip the download entirely");

        let _ = tokio::fs::remove_dir_all(&target_dir).await;
    }

    #[test]
    fn recognizes_vault_map_folders_vs_base_game_maps() {
        assert!(is_vault_map_folder("adaptive_gadostb.v0002"));
        assert!(is_vault_map_folder("FAF_Coop_Operation_Rescue.v0008"));
        assert!(!is_vault_map_folder("scmp_009"));
        assert!(!is_vault_map_folder("X1MP_002"));
        assert!(!is_vault_map_folder("no_version_suffix"));
        assert!(!is_vault_map_folder("trailing_dot_v"));
    }

    #[test]
    fn featured_mod_files_cannot_escape_the_install_root() {
        let root = Path::new("game");
        assert_eq!(
            safe_join_file(root, "gamedata", "units.nx2").unwrap(),
            root.join("gamedata").join("units.nx2")
        );
        for (group, name) in [
            ("..", "escape"),
            ("bin", "../escape"),
            ("/absolute", "escape"),
            ("bin", "C:\\escape"),
        ] {
            assert!(safe_join_file(root, group, name).is_err());
        }
    }

    #[tokio::test]
    async fn ensure_map_available_skips_the_network_entirely_for_base_maps() {
        let target_dir = std::env::temp_dir().join(format!("forge-basemap-{}", std::process::id()));
        let http = reqwest::Client::new();
        // An unreachable content_base proves no network call was attempted,
        // confirmed live (X1MP_002 → guaranteed 404, wrongly surfaced as a
        // "could not stage map" warning before this fix.
        ensure_map_available(&http, "http://127.0.0.1:1", &target_dir, "X1MP_002")
            .await
            .expect("base maps should be skipped, not looked up");
    }

    #[tokio::test]
    async fn cache_cleanup_and_inspection_works() {
        let temp_dir = std::env::temp_dir().join(format!("faf-test-cache-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        tokio::fs::create_dir_all(temp_dir.join("bin"))
            .await
            .unwrap();
        tokio::fs::write(temp_dir.join("bin").join("test.bin"), b"test data")
            .await
            .unwrap();

        let info = inspect_game_cache(&temp_dir, &[]).await;
        assert_eq!(info.total_files, 1);
        assert_eq!(info.total_size_bytes, 9.0);

        let removed = clean_expired_cache_files(&temp_dir, 0).await.unwrap();
        assert_eq!(removed, 0);

        clear_game_cache(&temp_dir).await.unwrap();
        let info_after = inspect_game_cache(&temp_dir, &[]).await;
        assert_eq!(info_after.total_files, 0);
        assert_eq!(info_after.total_size_bytes, 0.0);

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn cache_manifest_indexing_and_multi_version_inspection_works() {
        // Nested under a directory of its own, because `sync_version_folder`
        // writes to `cache_dir.parent()/versions/<entry name>`: with the cache
        // directly in the temp directory, every test in this file shared one
        // `versions` folder. This test and the staging one below both index an
        // entry called "FAF Develop (abcdef1)" holding a `lua.nx2`, so they
        // wrote the same file with different contents, in parallel, and
        // whichever finished second decided what the other one read back.
        let root =
            std::env::temp_dir().join(format!("faf-test-cache-manifest-{}", std::process::id()));
        let temp_dir = root.join("cache");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(temp_dir.join("bin"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(temp_dir.join("gamedata"))
            .await
            .unwrap();

        // Write files for build 3837 (exe + gamedata)
        tokio::fs::write(temp_dir.join("bin").join("md5_exe_3837"), b"12345678")
            .await
            .unwrap();
        tokio::fs::write(
            temp_dir.join("gamedata").join("md5_lua_3837"),
            b"1234567890",
        )
        .await
        .unwrap();

        // Write files for fafdevelop
        tokio::fs::write(
            temp_dir.join("gamedata").join("md5_dev_lua"),
            b"develop_lua_content",
        )
        .await
        .unwrap();

        let entry_3837 = CacheManifestEntry {
            featured_mod: "faf".to_string(),
            version: Some(3837),
            resolved_version: 3837,
            name: "FAF Build 3837".to_string(),
            url: Some("https://github.com/FAForever/fa/releases/tag/3837".to_string()),
            git_short_sha: None,
            signature: None,
            files: vec![
                CachedFileInfo {
                    group: "bin".to_string(),
                    md5: "md5_exe_3837".to_string(),
                    name: Some("ForgedAlliance.exe".to_string()),
                },
                CachedFileInfo {
                    group: "gamedata".to_string(),
                    md5: "md5_lua_3837".to_string(),
                    name: Some("lua.nx2".to_string()),
                },
            ],
            updated_at: 100,
        };
        save_cache_manifest_entry(&temp_dir, entry_3837);

        let entry_dev = CacheManifestEntry {
            featured_mod: "fafdevelop".to_string(),
            version: None,
            resolved_version: 0,
            name: "FAF Develop (abcdef1)".to_string(),
            url: Some("https://github.com/FAForever/fa/commits/abcdef1".to_string()),
            git_short_sha: Some("abcdef1".to_string()),
            signature: Some("abcdef1".to_string()),
            files: vec![CachedFileInfo {
                group: "gamedata".to_string(),
                md5: "md5_dev_lua".to_string(),
                name: Some("lua.nx2".to_string()),
            }],
            updated_at: 101,
        };
        save_cache_manifest_entry(&temp_dir, entry_dev);

        let info = inspect_game_cache(&temp_dir, &[]).await;
        assert_eq!(info.total_files, 3);
        assert_eq!(info.total_size_bytes, 37.0); // 37 bytes total
        assert_eq!(info.versions.len(), 2);

        let v_3837 = info
            .versions
            .iter()
            .find(|v| v.name == "FAF Build 3837")
            .unwrap();
        assert_eq!(v_3837.version, 3837);
        assert_eq!(v_3837.file_count, 2);
        assert_eq!(v_3837.size_bytes, 18.0);
        assert_eq!(
            v_3837.url.as_deref(),
            Some("https://github.com/FAForever/fa/releases/tag/3837")
        );

        let v_dev = info
            .versions
            .iter()
            .find(|v| v.name == "FAF Develop (abcdef1)")
            .unwrap();
        assert_eq!(v_dev.version, 0);
        assert_eq!(v_dev.file_count, 1);
        assert_eq!(v_dev.size_bytes, 19.0);
        assert_eq!(
            v_dev.url.as_deref(),
            Some("https://github.com/FAForever/fa/commits/abcdef1")
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn cache_manifest_stage_and_restore_replay_version_works() {
        // Its own root, for the shared `versions` folder described on the
        // manifest test above.
        let root = std::env::temp_dir().join(format!("forge-stage-test-{}", std::process::id()));
        let temp_dir = root.join("cache");
        let target_dir = root.join("target");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(temp_dir.join("bin"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(temp_dir.join("gamedata"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&target_dir).await.unwrap();

        // 10000-byte fake exe (large enough for version offset)
        let exe_bytes = vec![0u8; 10000];
        tokio::fs::write(temp_dir.join("bin").join("md5_exe_3837"), &exe_bytes)
            .await
            .unwrap();
        tokio::fs::write(
            temp_dir.join("gamedata").join("md5_lua_3837"),
            b"lua_content_3837",
        )
        .await
        .unwrap();
        tokio::fs::write(
            temp_dir.join("gamedata").join("md5_dev_lua"),
            b"lua_content_develop",
        )
        .await
        .unwrap();

        let entry_3837 = CacheManifestEntry {
            featured_mod: "faf".to_string(),
            version: Some(3837),
            resolved_version: 3837,
            name: "FAF Build 3837".to_string(),
            url: Some("https://github.com/FAForever/fa/releases/tag/3837".to_string()),
            git_short_sha: None,
            signature: None,
            files: vec![
                CachedFileInfo {
                    group: "bin".to_string(),
                    md5: "md5_exe_3837".to_string(),
                    name: Some("ForgedAlliance.exe".to_string()),
                },
                CachedFileInfo {
                    group: "gamedata".to_string(),
                    md5: "md5_lua_3837".to_string(),
                    name: Some("lua.nx2".to_string()),
                },
            ],
            updated_at: 100,
        };
        save_cache_manifest_entry(&temp_dir, entry_3837);

        let entry_dev = CacheManifestEntry {
            featured_mod: "fafdevelop".to_string(),
            version: None,
            resolved_version: 0,
            name: "FAF Develop (abcdef1)".to_string(),
            url: Some("https://github.com/FAForever/fa/commits/abcdef1".to_string()),
            git_short_sha: Some("abcdef1".to_string()),
            signature: Some("abcdef1".to_string()),
            files: vec![CachedFileInfo {
                group: "gamedata".to_string(),
                md5: "md5_dev_lua".to_string(),
                name: Some("lua.nx2".to_string()),
            }],
            updated_at: 500,
        };
        save_cache_manifest_entry(&temp_dir, entry_dev);

        // Test 1: Staging numbered version from cache
        let manifest = load_cache_manifest(&temp_dir);
        let e_3837 = manifest
            .entries
            .iter()
            .find(|e| e.resolved_version == 3837)
            .unwrap();
        stage_cached_version(&temp_dir, &target_dir, e_3837, &manifest).unwrap();

        assert!(target_dir.join("bin").join("ForgedAlliance.exe").is_file());
        assert!(target_dir.join("gamedata").join("lua.nx2").is_file());
        assert_eq!(
            tokio::fs::read(target_dir.join("gamedata").join("lua.nx2"))
                .await
                .unwrap(),
            b"lua_content_3837"
        );
        assert!(target_dir.join("fa_path.lua").is_file());
        assert!(target_dir.join(".faf_build.json").is_file());

        // Test 2: Resolve fafdevelop replay by exact commit SHA
        let http = reqwest::Client::new();
        let replay_info_sha = ReplayVersionInfo {
            mod_name: "fafdevelop".to_string(),
            game_version: None,
            git_sha: Some("abcdef1987654321".to_string()),
            git_short_sha: Some("abcdef1".to_string()),
            build_signature: Some("abcdef1".to_string()),
            version_name: Some("FAF Develop (abcdef1)".to_string()),
            launched_at: Some(510),
        };

        let warn = resolve_and_stage_replay_version(
            &http,
            "token",
            "http://127.0.0.1",
            &temp_dir,
            &target_dir,
            &replay_info_sha,
            "ForgedAlliance.exe",
        )
        .await
        .unwrap();

        assert_eq!(warn, None);
        assert_eq!(
            tokio::fs::read(target_dir.join("gamedata").join("lua.nx2"))
                .await
                .unwrap(),
            b"lua_content_develop"
        );

        // Test 3: Resolve fafdevelop replay by timestamp proximity (no git SHA in replay)
        let replay_info_time = ReplayVersionInfo {
            mod_name: "fafdevelop".to_string(),
            game_version: None,
            git_sha: None,
            git_short_sha: None,
            build_signature: None,
            version_name: None,
            launched_at: Some(505), // Close to updated_at = 500
        };

        let warn2 = resolve_and_stage_replay_version(
            &http,
            "token",
            "http://127.0.0.1",
            &temp_dir,
            &target_dir,
            &replay_info_time,
            "ForgedAlliance.exe",
        )
        .await
        .unwrap();

        assert_eq!(warn2, None);
        assert_eq!(
            tokio::fs::read(target_dir.join("gamedata").join("lua.nx2"))
                .await
                .unwrap(),
            b"lua_content_develop"
        );

        // The root, so the `versions` folder beside the cache goes with it.
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
