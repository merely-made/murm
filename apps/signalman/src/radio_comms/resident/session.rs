use super::*;

impl RadioComms {
    pub fn attach<'a>(
        &self,
        runtime: &'a mut Runtime,
    ) -> Result<ResidentSession<'a>, AdapterError> {
        let mut inner = self.lock()?;
        if inner.attached {
            return Err(backend("resident journal already attached"));
        }
        let Scope::Resident(scope) = &inner.book.scope else {
            return Err(backend(
                "stock companion journals cannot attach to a resident runtime",
            ));
        };
        if runtime
            .installed_sennet()
            .is_some_and(|s| s.address() != (scope.meshtastic_node, scope.meshtastic_channel))
            || runtime
                .installed_tucket()
                .is_some_and(|t| t.node().identity().pub_key != scope.meshcore_key)
        {
            return Err(backend(
                "resident public identity does not match journal realm",
            ));
        }
        let binding = runtime
            .installed_sennet()
            .and_then(|s| s.directed_binding());
        for record in &inner.book.records {
            for fact in &record.facts {
                if let RadioFact::MeshtasticPkiBinding {
                    peer,
                    local_public,
                    peer_public,
                } = fact
                    && binding.is_none_or(|b| {
                        b.peer != *peer
                            || b.local_public != *local_public
                            || b.peer_public != *peer_public
                    })
                {
                    return Err(backend(
                        "Meshtastic history is bound to another pinned PKI identity; select its original configuration or a new realm/journal",
                    ));
                }
            }
        }
        if runtime.installed_tucket().is_some() {
            runtime
                .restore_tucket_contacts(&inner.book.meshcore_contacts)
                .map_err(|e| {
                    backend(format!(
                        "cached MeshCore contacts cannot fit this runtime: {e:?}"
                    ))
                })?;
        }
        inner.attached = true;
        Ok(ResidentSession {
            owner: self.clone(),
            runtime,
            bindings: HashMap::new(),
            timestamp: HashMap::new(),
            transmissions: HashMap::new(),
        })
    }
}
impl ResidentSession<'_> {
    /// Advertise through the retained runtime and its ordinary physical work queue.
    pub fn advertise_tucket(
        &mut self,
        now: u64,
        unix_seconds: u32,
        name: &str,
    ) -> Result<(), AdapterError> {
        self.healthy()?;
        let data = tucket::advert::AdvertData::try_chat(name)
            .and_then(|d| d.encode())
            .ok_or_else(|| backend("resident advert name exceeds MeshCore bounds"))?;
        let report = self
            .runtime
            .advertise_tucket(now, unix_seconds, &data)
            .map_err(|e| backend(format!("{e:?}")))?;
        self.observe(now, report)
    }
    /// Full-key contact match, learned live or restored from its scoped journal.
    pub fn knows_tucket_peer(&self, key: &[u8; 32]) -> bool {
        self.runtime
            .installed_tucket()
            .and_then(|t| t.node().contact(key[0]))
            .is_some_and(|c| c.pub_key == *key)
    }
}
