use super::*;

enum ReadPoll {
    Waiting(StreamNextId),
    Working,
    Finished(Step),
}
enum ResumeState {
    Waiting(StreamNextId),
    Working,
    Completed,
}
impl Run {
    fn graph_consumer(&mut self, read: &PendingRead) -> Result<StreamConsumer, StreamError> {
        let (resource, ended) = self
            .graph_stream_handle(&read.work.reference, read.strand.region)
            .map_err(|_| StreamError::Stale)?;
        let handle = StreamHandle { resource };
        if ended {
            return Ok(StreamConsumer {
                handle,
                finished: true,
            });
        }
        let key = ConsumerKey {
            activation: read.strand.sequence,
            function: read.strand.function,
            owner: read.owner,
        };
        let stream = self.stream_resource(handle)?;
        if stream.claimed && stream.graph_consumer != Some(key) {
            return Err(StreamError::Claimed);
        }
        stream.claimed = true;
        stream.graph_consumer = Some(key);
        Ok(StreamConsumer {
            handle,
            finished: false,
        })
    }
    fn process_stream_read(
        &mut self,
        plan: &ControlPlan,
        read: &mut PendingRead,
        shared: &mut ActivationState,
    ) -> Result<ReadPoll, RunError> {
        let node = &plan.nodes()[read.owner.index()];
        if read.revoked {
            return Ok(ReadPoll::Finished(failure(
                node,
                stream_fault(StreamError::Stale),
            )));
        }
        let mut consumer = match self.graph_consumer(read) {
            Ok(consumer) => consumer,
            Err(error) => return Ok(ReadPoll::Finished(failure(node, stream_fault(error)))),
        };
        if let ReadMode::Collect { max } = read.work.mode {
            let outcome = self.collect_stream_inner(&mut consumer, max, &mut 1);
            return match outcome {
                Ok(StreamCollectPoll::Pending(id)) => Ok(ReadPoll::Waiting(id)),
                Ok(StreamCollectPoll::Working) => Ok(ReadPoll::Working),
                Ok(StreamCollectPoll::Complete(value)) => {
                    drop(read.work.hold.take());
                    Self::publish_match_value(
                        plan,
                        &self.layout,
                        shared,
                        read.owner,
                        Some(value),
                        self.options.limits.max_state_bytes,
                    )?;
                    Ok(ReadPoll::Finished((
                        node.then(),
                        ControlOutcome::Then,
                        None,
                    )))
                }
                Ok(StreamCollectPoll::Failed(fault)) => {
                    Ok(ReadPoll::Finished(failure(node, fault)))
                }
                Err(error) => {
                    let _ = self.close_stream(&mut consumer);
                    Ok(ReadPoll::Finished(failure(node, stream_fault(error))))
                }
            };
        }
        let outcome = match self.next_stream_inner(&mut consumer) {
            Ok(outcome) => outcome,
            Err(error) => {
                let _ = self.close_stream(&mut consumer);
                return Ok(ReadPoll::Finished(failure(node, stream_fault(error))));
            }
        };
        match outcome {
            StreamRead::Pending(id) => Ok(ReadPoll::Waiting(id)),
            StreamRead::Failed(fault) => Ok(ReadPoll::Finished(failure(node, fault))),
            outcome => match &read.work.mode {
                ReadMode::Next { boxed } => {
                    let value = match outcome {
                        StreamRead::End => Value::None,
                        StreamRead::Item(value) => {
                            if *boxed {
                                Value::Some(Arc::new(value))
                            } else {
                                value
                            }
                        }
                        _ => return Err(RunError::InvalidState),
                    };
                    drop(read.work.hold.take());
                    Self::publish_match_value(
                        plan,
                        &self.layout,
                        shared,
                        read.owner,
                        Some(value),
                        self.options.limits.max_state_bytes,
                    )?;
                    Ok(ReadPoll::Finished((
                        node.then(),
                        ControlOutcome::Then,
                        None,
                    )))
                }
                ReadMode::Iterate { index, parent } => {
                    let ControlOperation::ForEachItem {
                        body, completed, ..
                    } = node.operation()
                    else {
                        return Err(RunError::InvalidState);
                    };
                    match outcome {
                        StreamRead::End => Ok(ReadPoll::Finished((
                            *completed,
                            ControlOutcome::Completed,
                            None,
                        ))),
                        StreamRead::Item(value) => {
                            drop(read.work.hold.take());
                            if read.strand.stack.len() == self.options.limits.max_region_depth {
                                return Err(RunError::ResourceLimit);
                            }
                            let frame = IterationFrame::stream(
                                plan,
                                shared,
                                read.owner,
                                *index,
                                value,
                                parent.clone(),
                                self.options.limits.max_state_bytes,
                            )?;
                            read.strand
                                .stack
                                .push(Cursor::Stream(Box::new(StreamCursor {
                                    owner: read.owner,
                                    frame: Arc::new(frame),
                                    reference: read.work.reference.clone(),
                                    next: index.checked_add(1).ok_or(RunError::ResourceLimit)?,
                                })));
                            read.strand.transfer = Transfer::EnterBody(Some(read.owner));
                            read.strand.yielded = true;
                            Ok(ReadPoll::Finished((*body, ControlOutcome::Body, None)))
                        }
                        _ => Err(RunError::InvalidState),
                    }
                }
                ReadMode::Collect { .. } => Err(RunError::InvalidState),
            },
        }
    }
    pub(in crate::run) fn resume_stream_read(
        &mut self,
        plan: &ControlPlan,
    ) -> Result<(), RunError> {
        let token = *self
            .graph_streams
            .ready
            .front()
            .ok_or(RunError::InvalidState)?;
        let mut read = self
            .graph_streams
            .remove(token)
            .ok_or(RunError::InvalidState)?;
        let mut shared = self
            .activations
            .get_mut(read.strand.sequence)
            .and_then(Option::take)
            .ok_or(RunError::InvalidState)?;
        let result = self.process_stream_read(plan, &mut read, &mut shared);
        let result = match result {
            Ok(ReadPoll::Finished(step)) => {
                drop(read.work.hold.take());
                self.complete(plan, &mut read.strand, &mut shared, read.owner, step)
                    .map(|()| ResumeState::Completed)
            }
            Ok(ReadPoll::Waiting(id)) => Ok(ResumeState::Waiting(id)),
            Ok(ReadPoll::Working) => Ok(ResumeState::Working),
            Err(error) => Err(error),
        };
        *self
            .activations
            .get_mut(read.strand.sequence)
            .ok_or(RunError::InvalidState)? = Some(shared);
        match result? {
            ResumeState::Waiting(id) => {
                read.demand = Some(id);
                self.graph_streams.insert(read);
            }
            ResumeState::Working => {
                read.demand = None;
                self.graph_streams.insert(read);
            }
            ResumeState::Completed => match std::mem::take(&mut read.strand.transfer) {
                Transfer::EnterBody(owner) => self.enter_body(read.strand, owner)?,
                Transfer::Return(result) => self.leave_function(read.strand, result)?,
                Transfer::Fault => self.retire_failed(read.strand)?,
                Transfer::None => self.ready.push_back(read.strand.id, read.strand),
                _ => return Err(RunError::InvalidState),
            },
        }
        Ok(())
    }
    /// Iterator cursors in canceled strands close even when acquisition belongs
    /// to an ancestor. The retained item frame is independent of the producer.
    pub(in crate::run) fn close_stream_cursors(&mut self, strand: &Strand) -> Result<(), RunError> {
        for cursor in &strand.stack {
            if let Cursor::Stream(cursor) = cursor {
                self.close_graph_resource(&cursor.reference, strand.region)
                    .map_err(|_| RunError::InvalidState)?;
            }
        }
        Ok(())
    }
}
