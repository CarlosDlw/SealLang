//! Content-addressed storage transfer.
//!
//! Small blobs travel in bounded batches. A blob larger than the batch limit
//! uses the REAPI ByteStream service, so a code generation unit is not limited
//! by the gRPC message size. Every response is verified against its digest;
//! a mismatch is `E8122`, never a cache hit.

use std::sync::atomic::{AtomicU64, Ordering};

use seal_cas::Digest;

use crate::client::RemoteClient;
use crate::error::{RemoteError, RemoteResult};
use crate::proto;
use crate::types::{from_proto, to_proto_sized};

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const STREAM_CHUNK: usize = 64 * 1024;

static UPLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl RemoteClient {
    /// Returns the blobs absent from the remote CAS.
    ///
    /// The size is part of a REAPI digest, so callers must provide it. The
    /// empty blob is never uploaded.
    pub fn find_missing(&self, blobs: &[(Digest, u64)]) -> RemoteResult<Vec<Digest>> {
        let blobs: Vec<(Digest, u64)> = blobs
            .iter()
            .copied()
            .filter(|(digest, _)| *digest != Digest::of_bytes(b""))
            .collect();
        if blobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut missing = Vec::new();
        for chunk in blobs.chunks(1024) {
            let request = proto::FindMissingBlobsRequest {
                instance_name: self.instance_name().to_owned(),
                blob_digests: chunk
                    .iter()
                    .map(|(digest, size)| to_proto_sized(*digest, *size))
                    .collect::<RemoteResult<Vec<_>>>()?,
                digest_function: proto::digest_function::Value::Blake3 as i32,
            };
            let response = self.retry_unary(|| {
                let mut client =
                    proto::content_addressable_storage_client::ContentAddressableStorageClient::new(
                        self.channel(),
                    )
                    .max_decoding_message_size(MAX_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_MESSAGE_BYTES);
                self.unary(client.find_missing_blobs(self.request(request.clone())))
            })?;
            for digest in response.missing_blob_digests {
                missing.push(from_proto(&digest, None)?);
            }
        }
        Ok(missing)
    }

    /// Uploads one blob after verifying its local digest.
    pub fn upload_blob(&self, digest: Digest, bytes: &[u8]) -> RemoteResult<()> {
        if Digest::of_bytes(bytes) != digest {
            return Err(RemoteError::integrity(format!(
                "the local bytes for `{digest}` hash to `{}`",
                Digest::of_bytes(bytes)
            )));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        if self
            .find_missing(&[(digest, bytes.len() as u64)])?
            .is_empty()
        {
            return Ok(());
        }
        let capabilities = self.capabilities()?;
        let batch_limit = usize::try_from(capabilities.max_batch_bytes).unwrap_or(usize::MAX);
        if bytes.len() > batch_limit {
            return self.upload_stream(digest, bytes);
        }
        let compressed = capabilities
            .zstd
            .then(|| zstd::stream::encode_all(bytes, 3).ok())
            .flatten();
        let (data, compressor) = compressed.filter(|data| data.len() < bytes.len()).map_or(
            (bytes.to_vec(), proto::compressor::Value::Identity),
            |data| (data, proto::compressor::Value::Zstd),
        );
        let request = proto::BatchUpdateBlobsRequest {
            instance_name: self.instance_name().to_owned(),
            requests: vec![proto::batch_update_blobs_request::Request {
                digest: Some(to_proto_sized(digest, bytes.len() as u64)?),
                data,
                compressor: compressor as i32,
            }],
            digest_function: proto::digest_function::Value::Blake3 as i32,
        };
        let response = self.retry_unary(|| {
            let mut client =
                proto::content_addressable_storage_client::ContentAddressableStorageClient::new(
                    self.channel(),
                )
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES);
            self.unary(client.batch_update_blobs(self.request(request.clone())))
        })?;
        for result in response.responses {
            let status = result.status.unwrap_or_default();
            if status.code != 0 {
                return Err(RemoteError::integrity(format!(
                    "the remote CAS rejected `{digest}`: {}",
                    status.message
                )));
            }
        }
        Ok(())
    }

    /// Downloads and verifies one blob.
    pub fn download_blob(&self, digest: Digest, size: u64) -> RemoteResult<Vec<u8>> {
        if size == 0 {
            if digest != Digest::of_bytes(b"") {
                return Err(RemoteError::integrity(
                    "a zero-byte blob does not match its digest",
                ));
            }
            return Ok(Vec::new());
        }
        let capabilities = self.capabilities()?;
        let batch_limit = usize::try_from(capabilities.max_batch_bytes).unwrap_or(usize::MAX);
        if size > batch_limit as u64 {
            return self.download_stream(digest, size);
        }
        let request = proto::BatchReadBlobsRequest {
            instance_name: self.instance_name().to_owned(),
            digests: vec![to_proto_sized(digest, size)?],
            acceptable_compressors: if capabilities.zstd {
                vec![proto::compressor::Value::Zstd as i32]
            } else {
                Vec::new()
            },
            digest_function: proto::digest_function::Value::Blake3 as i32,
        };
        let response = self.retry_unary(|| {
            let mut client =
                proto::content_addressable_storage_client::ContentAddressableStorageClient::new(
                    self.channel(),
                )
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES);
            self.unary(client.batch_read_blobs(self.request(request.clone())))
        })?;
        let Some(result) = response.responses.into_iter().next() else {
            return Err(RemoteError::missing_output(format!(
                "the remote CAS returned no response for `{digest}`"
            )));
        };
        let status = result.status.unwrap_or_default();
        if status.code != 0 {
            return Err(RemoteError::missing_output(format!(
                "the remote CAS does not contain `{digest}`: {}",
                status.message
            )));
        }
        let bytes = match proto::compressor::Value::try_from(result.compressor)
            .unwrap_or(proto::compressor::Value::Identity)
        {
            proto::compressor::Value::Identity => result.data,
            proto::compressor::Value::Zstd => zstd::stream::decode_all(result.data.as_slice())
                .map_err(|error| {
                    RemoteError::integrity(format!("cannot decompress `{digest}`: {error}"))
                })?,
            other => {
                return Err(RemoteError::protocol(format!(
                    "the server used unsupported compression `{other:?}`"
                )));
            }
        };
        verify(digest, size, bytes)
    }

    fn upload_stream(&self, digest: Digest, bytes: &[u8]) -> RemoteResult<()> {
        self.retry_unary(|| self.upload_stream_once(digest, bytes))
    }

    fn upload_stream_once(&self, digest: Digest, bytes: &[u8]) -> RemoteResult<()> {
        let resource = upload_resource(self.instance_name(), digest, bytes.len() as u64);
        let count = bytes.len().div_ceil(STREAM_CHUNK);
        let requests: Vec<proto::google::bytestream::WriteRequest> = bytes
            .chunks(STREAM_CHUNK)
            .enumerate()
            .map(|(index, data)| proto::google::bytestream::WriteRequest {
                resource_name: if index == 0 {
                    resource.clone()
                } else {
                    String::new()
                },
                write_offset: (index * STREAM_CHUNK) as i64,
                finish_write: index + 1 == count,
                data: data.to_vec(),
            })
            .collect();
        self.runtime().block_on(async {
            let mut client = proto::google::bytestream::byte_stream_client::ByteStreamClient::new(
                self.channel(),
            )
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES);
            match tokio::time::timeout(
                self.config().timeout,
                client.write(self.request(tokio_stream::iter(requests))),
            )
            .await
            {
                Ok(Ok(response)) => {
                    let committed = response.into_inner().committed_size;
                    if committed >= 0 && committed as usize != bytes.len() {
                        return Err(RemoteError::integrity(format!(
                            "the remote stream committed {committed} of {} bytes for `{digest}`",
                            bytes.len()
                        )));
                    }
                    Ok(())
                }
                Ok(Err(status)) => Err(RemoteError::from(status)),
                Err(_) => Err(RemoteError::timeout("the remote upload timed out")),
            }
        })
    }

    fn download_stream(&self, digest: Digest, size: u64) -> RemoteResult<Vec<u8>> {
        self.retry_unary(|| self.download_stream_once(digest, size))
    }

    fn download_stream_once(&self, digest: Digest, size: u64) -> RemoteResult<Vec<u8>> {
        let resource = format!(
            "{}/blobs/{}/{}",
            self.instance_name(),
            digest.to_hex(),
            size
        );
        let expected = usize::try_from(size)
            .map_err(|_| RemoteError::configuration("the blob is too large for this host"))?;
        self.runtime().block_on(async {
            let mut client = proto::google::bytestream::byte_stream_client::ByteStreamClient::new(
                self.channel(),
            )
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES);
            let request = self.request(proto::google::bytestream::ReadRequest {
                resource_name: resource,
                read_offset: 0,
                read_limit: 0,
            });
            let mut stream = tokio::time::timeout(self.config().timeout, client.read(request))
                .await
                .map_err(|_| RemoteError::timeout("the remote read timed out"))?
                .map_err(RemoteError::from)?
                .into_inner();
            let mut bytes = Vec::with_capacity(expected);
            loop {
                match stream.message().await {
                    Ok(Some(chunk)) => {
                        if bytes.len().saturating_add(chunk.data.len()) > expected {
                            return Err(RemoteError::integrity(format!(
                                "`{digest}` is larger than its declared {size} bytes"
                            )));
                        }
                        bytes.extend_from_slice(&chunk.data);
                    }
                    Ok(None) => break,
                    Err(status) => return Err(RemoteError::from(status)),
                }
            }
            verify(digest, size, bytes)
        })
    }

    pub(crate) fn retry_unary<T>(
        &self,
        mut operation: impl FnMut() -> RemoteResult<T>,
    ) -> RemoteResult<T> {
        let mut last = None;
        for attempt in 0..=self.config().retries {
            match operation() {
                Ok(value) => return Ok(value),
                Err(error)
                    if attempt < self.config().retries
                        && matches!(
                            error.kind(),
                            crate::error::RemoteErrorKind::Transport
                                | crate::error::RemoteErrorKind::Timeout
                        ) =>
                {
                    last = Some(error);
                    std::thread::sleep(Self::retry_delay(attempt));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last.unwrap_or_else(|| RemoteError::transport("the remote request failed")))
    }
}

fn verify(digest: Digest, size: u64, bytes: Vec<u8>) -> RemoteResult<Vec<u8>> {
    if bytes.len() as u64 != size {
        return Err(RemoteError::integrity(format!(
            "`{digest}` has {} bytes, expected {size}",
            bytes.len()
        )));
    }
    if Digest::of_bytes(&bytes) != digest {
        return Err(RemoteError::integrity(format!(
            "the bytes for `{digest}` hash to `{}`",
            Digest::of_bytes(&bytes)
        )));
    }
    Ok(bytes)
}

fn upload_resource(instance: &str, digest: Digest, size: u64) -> String {
    let sequence = UPLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let prefix = if instance.is_empty() {
        String::new()
    } else {
        format!("{instance}/")
    };
    format!(
        "{prefix}uploads/seal-{}-{sequence}-{nonce}/blobs/{}/{}",
        std::process::id(),
        digest.to_hex(),
        size
    )
}

#[cfg(test)]
mod tests {
    use seal_cas::Digest;

    use super::verify;

    #[test]
    fn a_downloaded_blob_must_match_its_digest_and_size() {
        let bytes = b"remote bytes".to_vec();
        let digest = Digest::of_bytes(&bytes);
        let error = verify(digest, bytes.len() as u64, b"different bytes".to_vec())
            .expect_err("a digest mismatch is rejected");
        assert_eq!(error.code(), "E8122");
        let error = verify(digest, bytes.len() as u64 + 1, bytes.clone())
            .expect_err("a size mismatch is rejected");
        assert_eq!(error.code(), "E8122");
        assert_eq!(
            verify(digest, bytes.len() as u64, bytes).expect("the matching bytes are accepted"),
            b"remote bytes"
        );
    }
}
