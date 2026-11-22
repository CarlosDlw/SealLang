//! REAPI ByteStream for large identity blobs.

use std::pin::Pin;
use std::sync::Arc;

use seal_cas::Digest;
use seal_remote::proto;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::state::{State, status};

const CHUNK_BYTES: usize = 64 * 1024;
type ReadStream =
    Pin<Box<dyn Stream<Item = Result<proto::google::bytestream::ReadResponse, Status>> + Send>>;

/// The ByteStream service.
pub(crate) struct ByteStreamService {
    pub(crate) state: Arc<State>,
}

#[tonic::async_trait]
impl proto::google::bytestream::byte_stream_server::ByteStream for ByteStreamService {
    type ReadStream = ReadStream;

    async fn read(
        &self,
        request: Request<proto::google::bytestream::ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        let message = request.get_ref();
        self.state.authorize(&request, "").map_err(status)?;
        let (digest, size) =
            parse_read_resource(&message.resource_name, &self.state.config.instance_name)?;
        if size > self.state.config.max_blob_bytes as u64 {
            return Err(Status::invalid_argument(format!(
                "the blob `{digest}` exceeds the {} byte limit",
                self.state.config.max_blob_bytes
            )));
        }
        let bytes = self.state.store.get_blob(digest).map_err(status)?;
        let offset = usize::try_from(message.read_offset)
            .map_err(|_| Status::invalid_argument("the read offset is negative"))?;
        if offset > bytes.len() {
            return Err(Status::invalid_argument(
                "the read offset is beyond the blob",
            ));
        }
        let limit = if message.read_limit == 0 {
            bytes.len() - offset
        } else {
            let limit = usize::try_from(message.read_limit)
                .map_err(|_| Status::invalid_argument("the read limit is negative"))?;
            limit.min(bytes.len() - offset)
        };
        let slice = &bytes[offset..offset + limit];
        let responses: Vec<Result<proto::google::bytestream::ReadResponse, Status>> = slice
            .chunks(CHUNK_BYTES)
            .map(|chunk| {
                Ok(proto::google::bytestream::ReadResponse {
                    data: chunk.to_vec(),
                })
            })
            .collect();
        Ok(Response::new(Box::pin(tokio_stream::iter(responses))))
    }

    async fn write(
        &self,
        request: Request<tonic::Streaming<proto::google::bytestream::WriteRequest>>,
    ) -> Result<Response<proto::google::bytestream::WriteResponse>, Status> {
        self.state.authorize(&request, "").map_err(status)?;
        let mut stream = request.into_inner();
        let mut target: Option<(Digest, u64)> = None;
        let mut bytes = Vec::new();
        let mut finished = false;
        while let Some(message) = stream.message().await? {
            if finished {
                return Err(Status::invalid_argument(
                    "the upload continued after finish_write",
                ));
            }
            if let Some(first) = &target {
                if !message.resource_name.is_empty() {
                    let parsed = parse_write_resource(
                        &message.resource_name,
                        &self.state.config.instance_name,
                    )?;
                    if parsed != *first {
                        return Err(Status::invalid_argument(
                            "the upload resource changed mid-stream",
                        ));
                    }
                }
            } else {
                target = Some(parse_write_resource(
                    &message.resource_name,
                    &self.state.config.instance_name,
                )?);
            }
            let (_, size) = target.expect("the target was set");
            if message.write_offset < 0 || message.write_offset as usize != bytes.len() {
                return Err(Status::invalid_argument(
                    "the upload chunks are not contiguous",
                ));
            }
            if bytes.len().saturating_add(message.data.len()) > self.state.config.max_blob_bytes {
                return Err(Status::invalid_argument(format!(
                    "the upload exceeds the {} byte limit",
                    self.state.config.max_blob_bytes
                )));
            }
            bytes.extend_from_slice(&message.data);
            if message.finish_write {
                finished = true;
            }
            if bytes.len() as u64 > size {
                return Err(Status::invalid_argument(
                    "the upload is larger than its declared size",
                ));
            }
        }
        let (digest, size) =
            target.ok_or_else(|| Status::invalid_argument("the upload is empty"))?;
        if !finished {
            return Err(Status::invalid_argument(
                "the upload did not finish with finish_write",
            ));
        }
        self.state
            .store
            .put_blob(digest, size, &bytes)
            .map_err(status)?;
        Ok(Response::new(proto::google::bytestream::WriteResponse {
            committed_size: i64::try_from(size).unwrap_or(i64::MAX),
        }))
    }
}

fn parse_write_resource(resource: &str, instance: &str) -> Result<(Digest, u64), Status> {
    let rest = strip_instance(resource, instance, "uploads/")?;
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.len() != 4 || segments[1] != "blobs" {
        return Err(Status::invalid_argument(
            "the upload resource must end in `/uploads/<id>/blobs/<hash>/<size>`",
        ));
    }
    parse_parts(segments[2], segments[3])
}

fn parse_read_resource(resource: &str, instance: &str) -> Result<(Digest, u64), Status> {
    let rest = strip_instance(resource, instance, "blobs/")?;
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.len() != 2 {
        return Err(Status::invalid_argument(
            "the read resource must end in `/blobs/<hash>/<size>`",
        ));
    }
    parse_parts(segments[0], segments[1])
}

fn strip_instance<'a>(resource: &'a str, instance: &str, marker: &str) -> Result<&'a str, Status> {
    let resource = resource.trim_start_matches('/');
    let (prefix, rest) = if let Some(rest) = resource.strip_prefix(marker) {
        ("", rest)
    } else if let Some((prefix, rest)) = resource.split_once(&format!("/{marker}")) {
        (prefix, rest)
    } else {
        return Err(Status::invalid_argument("invalid ByteStream resource name"));
    };
    if !instance.is_empty() && !prefix.is_empty() && prefix != instance {
        return Err(Status::permission_denied(
            "the resource belongs to another instance",
        ));
    }
    Ok(rest)
}

fn parse_parts(hash: &str, size: &str) -> Result<(Digest, u64), Status> {
    let digest =
        Digest::parse(hash).map_err(|error| Status::invalid_argument(error.to_string()))?;
    let size = size
        .parse::<u64>()
        .map_err(|_| Status::invalid_argument("invalid blob size"))?;
    Ok((digest, size))
}
