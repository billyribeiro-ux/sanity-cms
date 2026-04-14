//! `POST /v1/assets/images/{dataset}` — upload an image asset.
//! `GET  /assets/images/{filename}`     — serve a previously uploaded asset.
//!
//! Sanity's upload contract is a raw octet-stream with the filename supplied
//! as a query parameter. We compute a SHA-1 of the body, write to a
//! content-addressed path on local disk, and upsert a `sanity.imageAsset`
//! document into the caller's dataset so GROQ queries can reference it.

use std::path::{Path as StdPath, PathBuf};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    routes::doc::map_repo_err,
    state::AppState,
};

use content_lake_core::{
    document::repo,
    events::types::{ContentLakeEvent, DocumentMutationEvent},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/assets/images/{dataset}", post(upload_image))
        .route("/assets/images/{filename}", get(serve_image))
}

#[derive(Debug, Deserialize)]
struct UploadQuery {
    #[serde(default)]
    filename: Option<String>,
}

#[tracing::instrument(skip(state, body))]
async fn upload_image(
    State(state): State<AppState>,
    Path(dataset): Path<String>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty upload body".into()));
    }

    let project = state.config().bootstrap_project.clone();
    let dataset_id = repo::resolve_dataset(state.pool(), &project, &dataset)
        .await
        .map_err(map_repo_err)?;

    // Sha1 the bytes; Sanity asset ids are content-addressed.
    let sha1_hex = {
        let mut hasher = Sha1::new();
        hasher.update(&body);
        let digest = hasher.finalize();
        hex_lower(&digest)
    };

    let original_filename = q.filename.unwrap_or_default();
    let ext = extension_from_filename(&original_filename).unwrap_or_else(|| "bin".to_string());
    let mime = mime_for_extension(&ext);
    let size = body.len();

    // Content-addressed disk path. Idempotent write: if present, skip.
    let assets_dir = state.config().assets_dir.clone();
    let images_dir = PathBuf::from(&assets_dir).join("images");
    tokio::fs::create_dir_all(&images_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("create assets dir: {e}")))?;

    let file_name = format!("{sha1_hex}.{ext}");
    let file_path = images_dir.join(&file_name);

    if !tokio::fs::try_exists(&file_path)
        .await
        .map_err(|e| ApiError::Internal(format!("stat asset: {e}")))?
    {
        tokio::fs::write(&file_path, &body)
            .await
            .map_err(|e| ApiError::Internal(format!("write asset: {e}")))?;
    }

    // Dimensions are cosmetic — we embed a placeholder so the id shape still
    // matches what Studio's asset resolver expects.
    let dims = "0x0";
    let asset_id = format!("image-{sha1_hex}-{dims}-{ext}");

    let url = format!(
        "{base}/assets/images/{file_name}",
        base = state.config().public_base_url.trim_end_matches('/'),
    );
    let path_str = format!("images/{file_name}");

    let now = Utc::now();
    let now_iso = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    // Build the sanity.imageAsset content. Store the same top-level fields
    // that Studio expects; system fields (_id/_rev/_type/timestamps) are
    // stripped by the repo layer.
    let content = json!({
        "_id": asset_id,
        "_type": "sanity.imageAsset",
        "_createdAt": now_iso,
        "_updatedAt": now_iso,
        "assetId": sha1_hex,
        "extension": ext,
        "mimeType": mime,
        "size": size,
        "sha1hash": sha1_hex,
        "url": url,
        "path": path_str,
        "originalFilename": original_filename,
    });

    let content_map = match content.clone() {
        Value::Object(m) => m,
        _ => unreachable!("constructed as object"),
    };

    // Upsert into the documents table (createOrReplace semantics keyed on id).
    let new_rev = short_rev();
    let stored = repo::upsert_document(
        state.pool(),
        dataset_id,
        &asset_id,
        "sanity.imageAsset",
        Some(&new_rev),
        &Value::Object(content_map),
    )
    .await
    .map_err(map_repo_err)?;

    // Broadcast a mutation event so live listeners see the new asset.
    let tx_id = Uuid::now_v7().as_simple().to_string();
    let result = crate::routes::doc::document_to_wire(stored.clone()).ok();
    let _ = state
        .event_bus()
        .publish(ContentLakeEvent::DocumentMutation(DocumentMutationEvent {
            dataset_id,
            document_id: asset_id.clone(),
            transaction_id: tx_id,
            transition: "update".to_string(),
            mutations: json!([{"createOrReplace": content}]),
            result,
        }));

    // Sanity's upload response shape.
    let document = json!({
        "_id": asset_id,
        "_type": "sanity.imageAsset",
        "_createdAt": stored.created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "_updatedAt": stored.updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "_rev": stored._rev,
        "assetId": sha1_hex,
        "extension": ext,
        "mimeType": mime,
        "size": size,
        "sha1hash": sha1_hex,
        "url": url,
        "path": path_str,
        "originalFilename": original_filename,
    });

    Ok(Json(json!({ "document": document })))
}

#[tracing::instrument(skip(state))]
async fn serve_image(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> ApiResult<Response> {
    // Defense-in-depth: reject any filename containing a path separator or
    // parent reference so we can't escape the images dir.
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(ApiError::BadRequest("invalid filename".into()));
    }

    let path: PathBuf = PathBuf::from(&state.config().assets_dir)
        .join("images")
        .join(&filename);

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::NotFound(format!("asset '{filename}' not found")));
        }
        Err(e) => return Err(ApiError::Internal(format!("read asset: {e}"))),
    };

    let ext = extension_from_filename(&filename).unwrap_or_else(|| "bin".to_string());
    let mime = mime_for_extension(&ext);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    Ok((StatusCode::OK, headers, bytes).into_response())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn extension_from_filename(name: &str) -> Option<String> {
    let p = StdPath::new(name);
    p.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
}

fn mime_for_extension(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn short_rev() -> String {
    let s = Uuid::now_v7().as_simple().to_string();
    s[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_extracts_lowercase() {
        assert_eq!(extension_from_filename("photo.PNG"), Some("png".into()));
        assert_eq!(extension_from_filename("doc.tar.gz"), Some("gz".into()));
        assert_eq!(extension_from_filename("noext"), None);
        assert_eq!(extension_from_filename(""), None);
    }

    #[test]
    fn mime_lookup_has_known_types() {
        assert_eq!(mime_for_extension("png"), "image/png");
        assert_eq!(mime_for_extension("jpeg"), "image/jpeg");
        assert_eq!(mime_for_extension("unknown"), "application/octet-stream");
    }

    #[test]
    fn hex_lower_is_correct() {
        assert_eq!(hex_lower(&[0, 1, 15, 16, 255]), "00010f10ff");
    }
}
