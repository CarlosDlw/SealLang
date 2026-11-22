//! REAPI Capabilities.

use std::sync::Arc;

use seal_remote::proto;
use tonic::{Request, Response, Status};

use crate::state::{State, status};

/// The capabilities service.
pub(crate) struct CapabilitiesService {
    pub(crate) state: Arc<State>,
}

#[tonic::async_trait]
impl proto::capabilities_server::Capabilities for CapabilitiesService {
    async fn get_capabilities(
        &self,
        request: Request<proto::GetCapabilitiesRequest>,
    ) -> Result<Response<proto::ServerCapabilities>, Status> {
        let message = request.get_ref();
        self.state
            .authorize(&request, &message.instance_name)
            .map_err(status)?;
        let cache = proto::CacheCapabilities {
            digest_functions: vec![proto::digest_function::Value::Blake3 as i32],
            action_cache_update_capabilities: Some(proto::ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            max_batch_total_size_bytes: self.state.config.max_batch_bytes as i64,
            supported_compressors: vec![proto::compressor::Value::Zstd as i32],
            supported_batch_update_compressors: vec![proto::compressor::Value::Zstd as i32],
            max_cas_blob_size_bytes: self.state.config.max_blob_bytes as i64,
        };
        let execution = if self.state.config.execution {
            Some(proto::ExecutionCapabilities {
                digest_function: proto::digest_function::Value::Blake3 as i32,
                exec_enabled: true,
                digest_functions: vec![proto::digest_function::Value::Blake3 as i32],
            })
        } else {
            Some(proto::ExecutionCapabilities {
                digest_function: proto::digest_function::Value::Blake3 as i32,
                exec_enabled: false,
                digest_functions: vec![proto::digest_function::Value::Blake3 as i32],
            })
        };
        Ok(Response::new(proto::ServerCapabilities {
            cache_capabilities: Some(cache),
            execution_capabilities: execution,
        }))
    }
}
