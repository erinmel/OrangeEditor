use super::*;
impl ReplayLog {
    pub fn stats(&self) -> ReplayStats {
        self.stats
    }
    pub fn seed(&self) -> u64 {
        self.seed
    }
    pub fn entropy_algorithm(&self) -> &'static str {
        crate::ENTROPY_ALGORITHM
    }
    pub fn graph_id(&self) -> orange_model::id::GraphId {
        self.plan.data().graph_id()
    }
    /// Re-execute a sealed prefix using its original plan, seed and exact host
    /// boundaries. No recorded Action callback or external executor is invoked.
    /// The archive must authenticate immutable references under host policy.
    pub fn begin_replay(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        archive: Arc<dyn ReplayArchive>,
    ) -> Result<ReplayPlayback, ReplayError> {
        if self.stats.incomplete || self.stats.version != 1 {
            return Err(ReplayError::Incomplete);
        }
        let mut run = Run::start(
            self.plan.clone(),
            document,
            registry,
            self.seed,
            self.options.clone(),
        )?;
        run.replay = Some(Session {
            mode: Mode::Replay {
                entries: self.entries.clone(),
                cursor: 0,
            },
            archive,
            failed: false,
        });
        Ok(ReplayPlayback {
            run,
            step_open: false,
        })
    }
    /// Convenience for hosts intentionally replaying the whole finite prefix in
    /// one call. Interactive hosts can use `begin_replay` and `step` instead.
    pub fn replay(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        archive: Arc<dyn ReplayArchive>,
    ) -> Result<ReplayedRun, ReplayError> {
        let mut playback = self.begin_replay(document, registry, archive)?;
        while playback.step()? {}
        playback.finish()
    }
}
impl ReplayPlayback {
    pub fn inspect(&self) -> &Run {
        &self.run
    }
    pub fn is_complete(&self) -> bool {
        !self.step_open
            && !self.run.pump_open
            && self.run.replay.as_ref().is_some_and(|s| {
                !s.failed
                    && matches!(&s.mode, Mode::Replay {entries,cursor} if *cursor==entries.len())
            })
    }
    /// Returns true after executing one command, false at the sealed end. Failure
    /// is terminal for this playback; no command may be retried out of order.
    pub fn step(&mut self) -> Result<bool, ReplayError> {
        if self.step_open {
            return Err(ReplayError::Incomplete);
        }
        self.step_open = true;
        let result = self.step_inner();
        self.step_open = false;
        if result.is_err()
            && let Some(session) = self.run.replay.as_mut()
        {
            session.failed = true;
        }
        result
    }
    fn step_inner(&mut self) -> Result<bool, ReplayError> {
        let run = &mut self.run;
        let entry = session(run)?.next()?;
        let Some(entry) = entry else {
            return Ok(false);
        };
        match entry {
            Entry::Pump {
                budget,
                result: Some(expected),
            } => equal(run.pump(budget), expected)?,
            Entry::Advance {
                now,
                result: Some(expected),
            } => equal(run.advance_to(now), expected)?,
            Entry::Stop {
                mode,
                result: Some(expected),
            } => equal(run.request_stop(mode), expected)?,
            Entry::Register {
                event,
                result: Some(expected),
            } => {
                let result = run.register_event_source(event).map(|s| s.id());
                equal(result, expected)?;
            }
            Entry::Revoke {
                source,
                result: Some(expected),
            } => {
                let token = source.token(run.id());
                equal(run.revoke_event_source(&token), expected)?;
            }
            Entry::Ingest {
                source,
                sequence,
                payload,
                wall,
                result: Some(expected),
            } => {
                let values = resolve(run, payload)?;
                if values.len() != 1 {
                    return Err(ReplayError::Diverged);
                }
                let token = source.token(run.id());
                equal(
                    run.ingest_event(&token, sequence, &values[0], wall),
                    expected,
                )?;
            }
            Entry::Trigger {
                node,
                payload,
                wall,
                result: Some(expected),
            } => {
                let values = payload.map(|r| resolve(run, r)).transpose()?;
                if values.as_ref().is_some_and(|v| v.len() != 1) {
                    return Err(ReplayError::Diverged);
                }
                equal(
                    run.trigger(node, values.as_ref().map(|v| &v[0]), wall),
                    expected,
                )?;
            }
            Entry::Take {
                result: Some(expected),
            } => {
                // Drop the inert request here; never return it to a live host.
                let result = run.take_effect().map(|r| (r.id.sequence, r.node, r.route));
                equal(result, expected)?;
            }
            Entry::Complete {
                sequence,
                foreign,
                outcome,
                result: Some(expected),
            } => {
                let id = EffectId {
                    run: if foreign { 0 } else { run.id() },
                    sequence,
                };
                let values = match outcome {
                    Ok(reference) => Some(resolve(run, reference)?),
                    Err(_) => None,
                };
                let outcome = match outcome {
                    Ok(_) => Ok(values.as_deref().ok_or(ReplayError::Diverged)?),
                    Err(fault) => Err(fault),
                };
                equal(run.complete_effect(id, outcome), expected)?;
            }
            _ => return Err(ReplayError::Incomplete),
        }
        if session(run)?.failed {
            return Err(ReplayError::Diverged);
        }
        Ok(true)
    }
    pub fn finish(mut self) -> Result<ReplayedRun, ReplayError> {
        if !self.is_complete() {
            return Err(ReplayError::Incomplete);
        }
        self.run.replay = None;
        Ok(ReplayedRun { run: self.run })
    }
}
fn session(run: &mut Run) -> Result<&mut Session, ReplayError> {
    run.replay.as_mut().ok_or(ReplayError::Incomplete)
}
fn resolve(run: &mut Run, reference: ReplayPayloadRef) -> Result<Arc<[Value]>, ReplayError> {
    session(run)?.resolve(reference)
}
fn equal<T: PartialEq>(actual: T, expected: T) -> Result<(), ReplayError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ReplayError::Diverged)
    }
}
