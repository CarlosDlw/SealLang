//! REAPI ContentAddressableStorage.

use std::io::Read;
use std::sync::Arc;

use seal_cas::Digest;
use seal_remote::proto;
use tonic::{Request, Response, Status};

use crate::error::ServerError;
use crate::state::{State, status};

/// The CAS service.
pub(crate) struct CasService {
    pub(crate) state: Arc<State>,
}

#[tonic::async_trait]
impl proto::content_addressable_storage_server::ContentAddressableStorage for CasService {
    async fn find_missing_blobs(
        &self,
        request: Request<proto::FindMissingBlobsRequest>,
    ) -> Result<Response<proto::FindMissingBlobsResponse>, Status> {
        let message = request.get_ref();
        self.state
            .authorize(&request, &message.instance_name)
            .map_err(status)?;
        let mut missing = Vec::new();
        for digest in &message.blob_digests {
            let (digest, size) = parse_digest(digest).map_err(status)?;
            if size > self.state.config.max_blob_bytes as u64 {
                return Err(Status::invalid_argument(format!(
                    "the blob `{digest}` exceeds the {} byte limit",
                    self.state.config.max_blob_bytes
                )));
            }
            if !self.state.store.contains(digest) {
                missing.push(wire_digest(digest, size));
            }
        }
        Ok(Response::new(proto::FindMissingBlobsResponse {
            missing_blob_digests: missing,
        }))
    }

    async fn batch_update_blobs(
        &self,
        request: Request<proto::BatchUpdateBlobsRequest>,
    ) -> Result<Response<proto::BatchUpdateBlobsResponse>, Status> {
        let message = request.get_ref();
        self.state
            .authorize(&request, &message.instance_name)
            .map_err(status)?;
        let total: usize = message.requests.iter().map(|item| item.data.len()).sum();
        if total > self.state.config.max_batch_bytes {
            return Err(Status::invalid_argument(format!(
                "the batch has {total} bytes, above the {} byte limit",
                self.state.config.max_batch_bytes
            )));
        }
        let mut responses = Vec::with_capacity(message.requests.len());
        for item in &message.requests {
            let response = match update_one(&self.state, item) {
                Ok(digest) => proto::batch_update_blobs_response::Response {
                    digest: Some(wire_digest(
                        Digest::parse(&digest.hash).unwrap_or_else(|_| Digest::of_bytes(b"")),
                        digest.size_bytes.max(0) as u64,
                    )),
                    status: Some(ok_status()),
                },
                Err(error) => proto::batch_update_blobs_response::Response {
                    digest: item.digest.clone(),
                    status: Some(error_status(&error)),
                },
            };
            responses.push(response);
        }
        Ok(Response::new(proto::BatchUpdateBlobsResponse { responses }))
    }

    async fn batch_read_blobs(
        &self,
        request: Request<proto::BatchReadBlobsRequest>,
    ) -> Result<Response<proto::BatchReadBlobsResponse>, Status> {
        let message = request.get_ref();
        self.state
            .authorize(&request, &message.instance_name)
            .map_err(status)?;
        let zstd = message
            .acceptable_compressors
            .contains(&(proto::compressor::Value::Zstd as i32));
        let total: u64 = message
            .digests
            .iter()
            .map(|digest| u64::try_from(digest.size_bytes).unwrap_or(0))
            .sum();
        if total > self.state.config.max_batch_bytes as u64 {
            return Err(Status::invalid_argument(format!(
                "the batch requests {total} bytes, above the {} byte limit",
                self.state.config.max_batch_bytes
            )));
        }
        let mut responses = Vec::with_capacity(message.digests.len());
        for wire in &message.digests {
            let response = match read_one(&self.state, wire, zstd) {
                Ok((digest, data, compressor)) => proto::batch_read_blobs_response::Response {
                    digest: Some(digest),
                    data,
                    status: Some(ok_status()),
                    compressor: compressor as i32,
                },
                Err(error) => proto::batch_read_blobs_response::Response {
                    digest: Some(wire.clone()),
                    data: Vec::new(),
                    status: Some(error_status(&error)),
                    compressor: proto::compressor::Value::Identity as i32,
                },
            };
            responses.push(response);
        }
        Ok(Response::new(proto::BatchReadBlobsResponse { responses }))
    }
}

fn update_one(
    state: &State,
    item: &proto::batch_update_blobs_request::Request,
) -> Result<proto::Digest, ServerError> {
    let wire = item
        .digest
        .as_ref()
        .ok_or_else(|| ServerError::invalid("a CAS update has no digest"))?;
    let (digest, size) = parse_digest(wire)?;
    if size > state.config.max_blob_bytes as u64 {
        return Err(ServerError::invalid(format!(
            "the blob `{digest}` exceeds the {} byte limit",
            state.config.max_blob_bytes
        )));
    }
    let data = match proto::compressor::Value::try_from(item.compressor)
        .unwrap_or(proto::compressor::Value::Identity)
    {
        proto::compressor::Value::Identity => item.data.clone(),
        proto::compressor::Value::Zstd => decompress(&item.data, state.config.max_blob_bytes)?,
        other => {
            return Err(ServerError::unsupported(format!(
                "the compressor `{other:?}` is not supported"
            )));
        }
    };
    state.store.put_blob(digest, size, &data)?;
    Ok(wire_digest(digest, size))
}

fn read_one(
    state: &State,
    wire: &proto::Digest,
    zstd: bool,
) -> Result<(proto::Digest, Vec<u8>, proto::compressor::Value), ServerError> {
    let (digest, size) = parse_digest(wire)?;
    if size > state.config.max_blob_bytes as u64 {
        return Err(ServerError::invalid(format!(
            "the blob `{digest}` exceeds the {} byte limit",
            state.config.max_blob_bytes
        )));
    }
    let bytes = state.store.get_blob(digest)?;
    let (data, compressor) = if zstd {
        match zstd::stream::encode_all(bytes.as_slice(), 3) {
            Ok(compressed) if compressed.len() < bytes.len() => {
                (compressed, proto::compressor::Value::Zstd)
            }
            _ => (bytes, proto::compressor::Value::Identity),
        }
    } else {
        (bytes, proto::compressor::Value::Identity)
    };
    Ok((wire_digest(digest, size), data, compressor))
}

fn decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, ServerError> {
    let decoder = zstd::stream::read::Decoder::new(data)
        .map_err(|error| ServerError::integrity(format!("cannot decode zstd data: {error}")))?;
    let mut output = Vec::new();
    decoder
        .take(limit as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|error| ServerError::integrity(format!("cannot decode zstd data: {error}")))?;
    if output.len() > limit {
        return Err(ServerError::integrity(format!(
            "the decompressed blob exceeds the {limit} byte limit"
        )));
    }
    Ok(output)
}

fn parse_digest(wire: &proto::Digest) -> Result<(Digest, u64), ServerError> {
    if wire.size_bytes < 0 {
        return Err(ServerError::invalid("a blob digest has a negative size"));
    }
    let digest = Digest::parse(&wire.hash).map_err(ServerError::from)?;
    Ok((digest, wire.size_bytes as u64))
}

pub(crate) fn wire_digest(digest: Digest, size: u64) -> proto::Digest {
    proto::Digest {
        hash: digest.to_hex(),
        size_bytes: i64::try_from(size).unwrap_or(i64::MAX),
    }
}

pub(crate) fn ok_status() -> proto::google::rpc::Status {
    proto::google::rpc::Status {
        code: 0,
        message: String::new(),
        details: Vec::new(),
    }
}

pub(crate) fn error_status(error: &ServerError) -> proto::google::rpc::Status {
    let code = match error.status() {
        tonic::Code::NotFound => 5,
        tonic::Code::InvalidArgument => 3,
        tonic::Code::Unimplemented => 12,
        tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => 7,
        _ => 13,
    };
    proto::google::rpc::Status {
        code,
        message: error.to_string(),
        details: Vec::new(),
    }
}
