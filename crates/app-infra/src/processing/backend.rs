use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

use crate::{AppInfraError, Result};

use super::{ProcessingJob, ProcessingResultDraft, ProcessingStore};

#[async_trait]
pub trait ProcessorBackend: Send + Sync {
    fn processor(&self) -> &'static str;

    async fn process(
        &self,
        store: &ProcessingStore,
        job: &ProcessingJob,
    ) -> Result<ProcessingResultDraft>;
}

#[derive(Clone, Default)]
pub struct ProcessorRegistry {
    backends: HashMap<String, Arc<dyn ProcessorBackend>>,
}

impl ProcessorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<B>(mut self, backend: B) -> Self
    where
        B: ProcessorBackend + 'static,
    {
        self.backends
            .insert(backend.processor().to_string(), Arc::new(backend));
        self
    }

    pub fn register_arc(mut self, backend: Arc<dyn ProcessorBackend>) -> Self {
        self.backends
            .insert(backend.processor().to_string(), backend);
        self
    }

    pub fn backend_for(&self, processor: &str) -> Result<Arc<dyn ProcessorBackend>> {
        self.backends
            .get(processor)
            .cloned()
            .ok_or_else(|| AppInfraError::UnknownProcessor(processor.to_string()))
    }
}
