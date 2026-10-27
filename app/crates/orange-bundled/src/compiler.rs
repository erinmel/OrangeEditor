//! Trusted native adapter. The compiler itself has no bundled-provider dependency.
use crate::{MathPackage, Operation, ResolvedConversion, ResolvedKernel, resolve_core_conversion};
use orange_compiler::{KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveError};
use orange_model::catalog::{ProviderKey, RegistrySnapshot, TypeArgs, intrinsic_id};
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
    fn resolve(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        if node.definition.id.as_str() == intrinsic_id::CONVERT
            && node.definition.version == 1
            && node.config_version == 1
            && node.provider.is_none()
        {
            let config =
                ConverterConfig::from_config(&node.config).map_err(|_| ResolveError::Contract)?;
            let conversion = resolve_core_conversion(snapshot, &config, &ConverterProvider::Core)
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
        let provider = node
            .provider
            .as_ref()
            .and_then(|binding| ProviderKey::of(document, binding))
            .ok_or(ResolveError::Unavailable)?;
        let operation = Operation::from_definition_id(node.definition.id.as_str())
            .ok_or(ResolveError::Unavailable)?;
        let inputs = operation.input_ports();
        let args: TypeArgs = if !operation.is_numeric() {
            Default::default()
        } else {
            let ty = signature
                .ports
                .iter()
                .find(|port| port.port_id.as_str() == inputs[0])
                .and_then(|port| port.ty.clone())
                .ok_or(ResolveError::Contract)?;
            [("T".into(), ty)].into()
        };
        let kernel = MathPackage::resolve(
            self,
            snapshot,
            &provider,
            &node.definition,
            node.config_version,
            &args,
            &node.config,
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
