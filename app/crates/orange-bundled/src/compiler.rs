//! Trusted native adapter. The compiler itself has no bundled-provider dependency.
use crate::{MathPackage, Operation, ResolvedConversion, ResolvedKernel, resolve_core_conversion};
use orange_compiler::{
    BindingRequest, KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveError,
};
use orange_model::catalog::{RegistrySnapshot, TypeArgs, intrinsic_id};
use orange_model::conversion::{ConverterConfig, ConverterProvider};
use orange_model::document::{NodeInstance, SignatureSnapshot};
use orange_model::id::PortKey;
use orange_model::{GraphDocument, Value};
use std::sync::Arc;

fn ports(names: &[&str]) -> Result<Box<[PortKey]>, ResolveError> {
    names
        .iter()
        .map(|name| name.parse().map_err(|_| ResolveError::Contract))
        .collect()
}

impl PureKernelResolver for MathPackage {
    /// Compatibility entry: extracts the exact provider once, then binds narrowly.
    fn resolve(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        self.resolve_binding(&BindingRequest::new(document, node, signature, snapshot)?)
    }

    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        let definition = request.definition();
        if definition.id.as_str() == intrinsic_id::CONVERT
            && definition.version == 1
            && request.config_version() == 1
            && request.provider().is_none()
        {
            let config = ConverterConfig::from_config(request.config())
                .map_err(|_| ResolveError::Contract)?;
            let conversion = resolve_core_conversion(
                request.snapshot().clone(),
                &config,
                &ConverterProvider::Core,
            )
            .map_err(|_| ResolveError::Unavailable)?;
            return Ok(KernelBinding {
                inputs: ports(&["value"])?,
                outputs: ports(if conversion.lossy() {
                    &["result", "loss"]
                } else {
                    &["result"]
                })?,
                kernel: Arc::new(Native::Conversion(conversion)),
            });
        }
        let provider = request.provider().ok_or(ResolveError::Unavailable)?;
        let operation = Operation::from_definition_id(definition.id.as_str())
            .ok_or(ResolveError::Unavailable)?;
        let inputs = operation.input_ports();
        let args: TypeArgs = if !operation.is_numeric() {
            Default::default()
        } else {
            let ty = request
                .signature()
                .ports
                .iter()
                .find(|port| port.port_id.as_str() == inputs[0])
                .and_then(|port| port.ty.clone())
                .ok_or(ResolveError::Contract)?;
            [("T".into(), ty)].into()
        };
        let kernel = MathPackage::resolve(
            self,
            request.snapshot().clone(),
            provider,
            definition,
            request.config_version(),
            &args,
            request.config(),
        )
        .map_err(|_| ResolveError::Unavailable)?;
        Ok(KernelBinding {
            inputs: ports(inputs)?,
            outputs: ports(&[operation.output_port()])?,
            kernel: Arc::new(Native::Math(kernel)),
        })
    }
}

#[derive(Debug)]
enum Native {
    Math(ResolvedKernel),
    Conversion(ResolvedConversion),
}
const CONTRACT: KernelFault = KernelFault {
    code: "core/InternalFault",
    message_key: "core/kernel-input-contract",
    message: "Kernel inputs violate the resolved contract.",
};
impl PureKernel for Native {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        match self {
            Self::Math(kernel) => {
                let [output] = outputs else {
                    return Err(CONTRACT);
                };
                *output = kernel.evaluate(inputs).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
                    message: match error {
                        crate::KernelError::NumericOverflow => "Numeric arithmetic overflowed.",
                        crate::KernelError::DivideByZero => "Division requires a nonzero divisor.",
                        crate::KernelError::NonFinite => {
                            "Finite arithmetic requires finite inputs and results."
                        }
                        crate::KernelError::InputContract => CONTRACT.message,
                    },
                })?;
            }
            Self::Conversion(conversion) => {
                let [input] = inputs else {
                    return Err(CONTRACT);
                };
                let value = conversion.evaluate(input).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
                    message: match error {
                        crate::ConversionError::OutOfRange => {
                            "The value is outside the target conversion range."
                        }
                        crate::ConversionError::InputContract => {
                            "Converter input violates the resolved contract."
                        }
                    },
                })?;
                match (outputs, value.loss) {
                    ([result], None) => *result = value.result,
                    ([result, loss], Some(metadata)) => {
                        *result = value.result;
                        *loss = metadata;
                    }
                    _ => return Err(CONTRACT),
                }
            }
        }
        Ok(())
    }
}
