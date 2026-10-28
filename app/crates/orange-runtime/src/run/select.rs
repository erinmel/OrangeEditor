//! Branch, Switch and Match Result route on one consumed value. They write
//! only the activation's execution outputs; run cells and scheduling are untouched.
use super::*;

impl Run {
    pub(super) fn select(
        plan: &ControlPlan,
        (layout, limit): (&StateLayout, usize),
        ix: ControlIx,
        shared: &mut ActivationState,
        value: Option<Value>,
    ) -> Result<Step, RunError> {
        let node = &plan.nodes()[ix.index()];
        let contract = DataError::KernelContract { node: node.node() };
        Ok(match (node.operation(), value) {
            (ControlOperation::Branch { when_true, .. }, Some(Value::Bool(true))) => {
                (*when_true, ControlOutcome::True, None)
            }
            (ControlOperation::Branch { when_false, .. }, Some(Value::Bool(false))) => {
                (*when_false, ControlOutcome::False, None)
            }
            (ControlOperation::Branch { .. }, Some(_)) => return Err(contract.into()),
            (ControlOperation::Switch { cases, default, .. }, Some(value)) => {
                let key = SwitchKey::from_value(&value).ok_or(contract)?;
                match cases.binary_search_by(|case| case.key().cmp(&key)) {
                    Ok(index) => (
                        cases[index].target(),
                        ControlOutcome::Case(cases[index].instance()),
                        None,
                    ),
                    Err(_) => (*default, ControlOutcome::Default, None),
                }
            }
            (ControlOperation::MatchResult { err, .. }, Some(Value::Err(error))) => {
                Self::publish_match_value(plan, layout, shared, ix, None, limit)?;
                for &i in plan.error_targets(ix) {
                    shared.outputs[layout.outputs[i].expect("checked output")] =
                        Some(FoldedValue::Value(Value::Err(error.clone())));
                }
                (*err, ControlOutcome::MatchedErr, None)
            }
            (ControlOperation::MatchResult { boxed, ok, .. }, Some(value)) => {
                let value = match (boxed, value) {
                    (true, Value::Ok(value)) => value.as_ref().clone(),
                    (false, value) => value,
                    _ => return Err(contract.into()),
                };
                Self::publish_match_value(plan, layout, shared, ix, Some(value), limit)?;
                for &i in plan.error_targets(ix) {
                    shared.outputs[layout.outputs[i].expect("checked output")] = None;
                }
                (*ok, ControlOutcome::MatchedOk, None)
            }
            _ => return Err(RunError::InvalidState),
        })
    }
}
