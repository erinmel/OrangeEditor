//! Independent bounded source interpreter using ordinary Rust futures and a
//! deliberately small polling executor. No compiled indices, runtime cursors,
//! owner tables or production join/cancellation helpers are used.
use super::*;
use orange_model::catalog::RegistrySnapshot;
use orange_model::id::{NodeId, PortKey, VariableId};
use orange_model::validation::ValidationAnalysis;
use orange_model::value::literal_codec;
use orange_model::{GraphDocument, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

type Job<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
type Body<'a> = Pin<Box<dyn Future<Output = bool> + 'a>>;
struct Cancel {
    flag: Cell<bool>,
    parent: Option<Rc<Cancel>>,
}
impl Cancel {
    fn cancelled(&self) -> bool {
        self.flag.get() || self.parent.as_ref().is_some_and(|p| p.cancelled())
    }
}
pub struct ForkReference<'a> {
    doc: &'a GraphDocument,
    snapshot: &'a RegistrySnapshot,
    data: RefCell<reference::Reference<'a>>,
    now: Cell<u64>,
    wakeup: Cell<Option<u64>>,
    steps: Cell<usize>,
    root: Rc<Cancel>,
    pub vars: RefCell<BTreeMap<VariableId, Option<Value>>>,
    pub observations: RefCell<Vec<(u64, NodeId, &'static str)>>,
    pub faults: RefCell<Vec<(NodeId, NodeId, &'static str)>>,
}
impl<'a> ForkReference<'a> {
    pub fn new(
        doc: &'a GraphDocument,
        snapshot: &'a RegistrySnapshot,
        analysis: &'a ValidationAnalysis,
        seed: u64,
    ) -> Self {
        Self {
            doc,
            snapshot,
            data: RefCell::new(reference::Reference::seeded(
                doc, snapshot, analysis, seed, 0,
            )),
            now: Cell::new(0),
            wakeup: Cell::new(None),
            steps: Cell::new(0),
            root: Rc::new(Cancel {
                flag: Cell::new(false),
                parent: None,
            }),
            vars: RefCell::new(
                doc.variables()
                    .iter()
                    .map(|(id, v)| {
                        (
                            *id,
                            v.init.as_ref().map(|l| {
                                literal_codec::decode(l, snapshot.types().as_ref()).unwrap()
                            }),
                        )
                    })
                    .collect(),
            ),
            observations: RefCell::new(vec![]),
            faults: RefCell::new(vec![]),
        }
    }
    fn target(&self, id: NodeId, port: &str) -> Option<NodeId> {
        let port: PortKey = port.parse().unwrap();
        self.doc
            .root()
            .edges
            .values()
            .find(|e| e.from.node == id && e.from.port == port)
            .map(|e| e.to.node)
    }
    fn observe(&self, id: NodeId, label: &'static str) {
        self.observations
            .borrow_mut()
            .push((self.now.get(), id, label));
    }
    fn input(&self, id: NodeId, port: &str) -> reference::Outcome {
        let mut data = self.data.borrow_mut();
        data.begin_epoch();
        for n in self
            .doc
            .root()
            .nodes
            .values()
            .filter(|n| n.definition.id.as_str() == "core/get-variable")
        {
            let variable: VariableId = n.config["variable"].as_str().unwrap().parse().unwrap();
            data.supply(
                n.id,
                "value".parse().unwrap(),
                self.vars.borrow()[&variable]
                    .clone()
                    .ok_or(reference::Fault {
                        source: n.id,
                        code: "core/UninitializedVariable",
                    }),
            );
        }
        let port: PortKey = port.parse().unwrap();
        if let Some(e) = self
            .doc
            .root()
            .edges
            .values()
            .find(|e| e.to.node == id && e.to.port == port)
        {
            data.source(e.from.node, &e.from.port)
        } else {
            Ok(literal_codec::decode(
                &self.doc.root().nodes[&id].inputs[&port],
                self.snapshot.types().as_ref(),
            )
            .unwrap())
        }
    }
    fn body<'b>(
        &'b self,
        jobs: Rc<RefCell<Vec<Job<'b>>>>,
        mut pc: Option<NodeId>,
        cancel: Rc<Cancel>,
        depth: usize,
    ) -> Body<'b> {
        Box::pin(async move {
            assert!(depth < 16);
            let outstanding = Rc::new(Cell::new(0usize));
            let mut cleanup = false;
            let mut success = true;
            while let Some(id) = pc {
                self.steps.set(self.steps.get() + 1);
                assert!(self.steps.get() < 10000);
                if cancel.cancelled() && !cleanup {
                    success = false;
                    break;
                }
                let node = &self.doc.root().nodes[&id];
                pc = match node.definition.id.as_str() {
                    "core/on-start" => self.target(id, "then"),
                    "core/sequence" => {
                        for key in &node.variadic[&"then".parse().unwrap()].order {
                            if !self
                                .body(
                                    jobs.clone(),
                                    self.target(id, &format!("then#{key}")),
                                    cancel.clone(),
                                    depth + 1,
                                )
                                .await
                            {
                                success = false;
                                break;
                            }
                        }
                        if !success {
                            break;
                        }
                        None
                    }
                    "core/count" => {
                        let Value::I64(count) = self.input(id, "count").unwrap() else {
                            panic!()
                        };
                        for _ in 0..count {
                            if !self
                                .body(
                                    jobs.clone(),
                                    self.target(id, "body"),
                                    cancel.clone(),
                                    depth + 1,
                                )
                                .await
                            {
                                success = false;
                                break;
                            }
                        }
                        if !success {
                            break;
                        }
                        self.target(id, "completed")
                    }
                    "core/fork" => {
                        let keys = &node.variadic[&"branch".parse().unwrap()].order;
                        let done = Rc::new(Cell::new(0usize));
                        let winner = Rc::new(Cell::new(None));
                        let mut tokens = vec![];
                        for (index, key) in keys.iter().enumerate() {
                            let child = Rc::new(Cancel {
                                flag: Cell::new(false),
                                parent: Some(cancel.clone()),
                            });
                            tokens.push(child.clone());
                            let (done, winner, outstanding) =
                                (done.clone(), winner.clone(), outstanding.clone());
                            outstanding.set(outstanding.get() + 1);
                            let target = self.target(id, &format!("branch#{key}"));
                            let child_jobs = jobs.clone();
                            jobs.borrow_mut().push(Box::pin(async move {
                                let ok = self.body(child_jobs, target, child, depth + 1).await;
                                if ok && winner.get().is_none() {
                                    winner.set(Some(index));
                                }
                                done.set(done.get() + 1);
                                outstanding.set(outstanding.get() - 1);
                            }));
                        }
                        let mode = node.config["join"].as_str().unwrap();
                        if mode != "detach" {
                            poll_fn(|_| {
                                if cancel.cancelled()
                                    || if mode == "all" {
                                        done.get() == keys.len()
                                    } else {
                                        winner.get().is_some()
                                    }
                                {
                                    Poll::Ready(())
                                } else {
                                    Poll::Pending
                                }
                            })
                            .await;
                        }
                        if cancel.cancelled() {
                            success = false;
                            break;
                        }
                        if mode == "race" {
                            for (i, token) in tokens.iter().enumerate() {
                                if Some(i) != winner.get() {
                                    token.flag.set(true);
                                }
                            }
                        }
                        self.target(id, "completed")
                    }
                    "core/wait" => {
                        let Value::Duration(duration) = self.input(id, "duration").unwrap() else {
                            panic!()
                        };
                        let at = self.now.get() + duration as u64;
                        poll_fn(|_| {
                            if cancel.cancelled() || self.now.get() >= at {
                                Poll::Ready(())
                            } else {
                                self.wakeup
                                    .set(Some(self.wakeup.get().map_or(at, |old| old.min(at))));
                                Poll::Pending
                            }
                        })
                        .await;
                        if cancel.cancelled() {
                            self.observe(id, "cancelled");
                            cleanup = true;
                            self.target(id, "cancelled")
                        } else {
                            self.observe(id, "then");
                            self.target(id, "then")
                        }
                    }
                    "core/set-variable" => match self.input(id, "value") {
                        Ok(value) => {
                            let variable =
                                node.config["variable"].as_str().unwrap().parse().unwrap();
                            self.vars.borrow_mut().insert(variable, Some(value));
                            self.observe(id, "then");
                            self.target(id, "then")
                        }
                        Err(f) => {
                            self.faults.borrow_mut().push((id, f.source, f.code));
                            let target = self.target(id, "failed");
                            if target.is_none() {
                                self.root.flag.set(true);
                                success = false;
                            }
                            target
                        }
                    },
                    name => panic!("unsupported source oracle control {name}"),
                };
            }
            poll_fn(|_| {
                if outstanding.get() == 0 {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await;
            success
        })
    }
    pub fn run(&self) {
        let jobs: Rc<RefCell<Vec<Job<'_>>>> = Rc::new(RefCell::new(vec![]));
        let entry = self
            .doc
            .root()
            .nodes
            .values()
            .find(|n| n.definition.id.as_str() == "core/on-start")
            .unwrap()
            .id;
        let root_jobs = jobs.clone();
        jobs.borrow_mut().push(Box::pin(async move {
            self.body(root_jobs, Some(entry), self.root.clone(), 0)
                .await;
        }));
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..20000 {
            let round = std::mem::take(&mut *jobs.borrow_mut());
            if round.is_empty() {
                return;
            }
            self.wakeup.set(None);
            let mut completed = 0;
            let mut pending = vec![];
            for mut job in round {
                if job.as_mut().poll(&mut cx).is_ready() {
                    completed += 1;
                } else {
                    pending.push(job);
                }
            }
            let spawned = !jobs.borrow().is_empty();
            pending.append(&mut *jobs.borrow_mut());
            *jobs.borrow_mut() = pending;
            if completed == 0
                && !spawned
                && let Some(at) = self.wakeup.get()
            {
                self.now.set(at);
            }
        }
        panic!("source executor exceeded bounded polling work");
    }
}
