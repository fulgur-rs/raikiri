use weedle::argument::Argument;
use weedle::attribute::{ExtendedAttribute, IdentifierOrString};
use weedle::interface::InterfaceMember;
use weedle::types::{IntegerType, NonAnyType, ReturnType, SingleType, Type};
use weedle::{Definition, Parse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    String,
    NullableString,
    NullableNode,
    U16,
    Bool,
    Undefined,
}

#[derive(Debug)]
pub struct Member {
    pub name: String,
    pub getter: bool,
    pub arguments: Vec<Ty>,
    pub result: Ty,
}

#[derive(Debug)]
pub struct Interface {
    pub name: String,
    pub parent: Option<String>,
    pub members: Vec<Member>,
}

fn ty(value: &Type<'_>) -> Result<Ty, String> {
    let Type::Single(SingleType::NonAny(value)) = value else {
        return Err("unsupported type".into());
    };
    match value {
        NonAnyType::DOMString(value) => Ok(if value.q_mark.is_some() {
            Ty::NullableString
        } else {
            Ty::String
        }),
        NonAnyType::Identifier(value) if value.type_.0 == "Node" && value.q_mark.is_some() => {
            Ok(Ty::NullableNode)
        }
        NonAnyType::Integer(value)
            if value.q_mark.is_none()
                && matches!(value.type_, IntegerType::Short(ref v) if v.unsigned.is_some()) =>
        {
            Ok(Ty::U16)
        }
        NonAnyType::Boolean(value) if value.q_mark.is_none() => Ok(Ty::Bool),
        _ => Err("unsupported type".into()),
    }
}

pub fn parse(source: &str) -> Result<Vec<Interface>, String> {
    let (remaining, definitions) =
        weedle::Definitions::parse(source).map_err(|e| format!("IDL parse: {e:?}"))?;
    if !remaining.trim().is_empty() {
        return Err(format!("unparsed IDL: {remaining}"));
    }
    let mut interfaces = Vec::new();
    for definition in definitions {
        let Definition::Interface(interface) = definition else {
            return Err("only interfaces supported".into());
        };
        let attributes = interface
            .attributes
            .as_ref()
            .ok_or("Exposed=Window required")?;
        if attributes.body.list.len() != 1
            || !matches!(&attributes.body.list[0], ExtendedAttribute::Ident(a) if a.lhs_identifier.0 == "Exposed" && matches!(a.rhs, IdentifierOrString::Identifier(i) if i.0 == "Window"))
        {
            return Err("only Exposed=Window supported".into());
        }
        let mut members = Vec::new();
        for member in interface.members.body {
            let member = match member {
                InterfaceMember::Attribute(a)
                    if a.readonly.is_some()
                        && a.modifier.is_none()
                        && a.attributes.is_none()
                        && a.type_.attributes.is_none() =>
                {
                    Member {
                        name: a.identifier.0.to_owned(),
                        getter: true,
                        arguments: Vec::new(),
                        result: ty(&a.type_.type_)?,
                    }
                }
                InterfaceMember::Operation(o)
                    if o.modifier.is_none() && o.special.is_none() && o.attributes.is_none() =>
                {
                    let mut arguments = Vec::new();
                    for arg in o.args.body.list {
                        let Argument::Single(arg) = arg else {
                            return Err("variadic arguments unsupported".into());
                        };
                        if arg.optional.is_some()
                            || arg.default.is_some()
                            || arg.attributes.is_some()
                            || arg.type_.attributes.is_some()
                        {
                            return Err("optional/default/extended arguments unsupported".into());
                        }
                        let arg_ty = ty(&arg.type_.type_)?;
                        if !matches!(arg_ty, Ty::String | Ty::NullableNode) {
                            return Err("unsupported argument type".into());
                        }
                        arguments.push(arg_ty);
                    }
                    Member {
                        name: o
                            .identifier
                            .ok_or("anonymous operation unsupported")?
                            .0
                            .to_owned(),
                        getter: false,
                        arguments,
                        result: match o.return_type {
                            ReturnType::Undefined(_) => Ty::Undefined,
                            ReturnType::Type(t) => ty(&t)?,
                        },
                    }
                }
                _ => return Err("unsupported member".into()),
            };
            if member.result == Ty::String {
                return Err("non-nullable string returns unsupported".into());
            }
            if members.iter().any(|m: &Member| m.name == member.name) {
                return Err("overloads unsupported".into());
            }
            members.push(member);
        }
        let name = interface.identifier.0.to_owned();
        if !matches!(name.as_str(), "Node" | "Element") {
            return Err("prototype only supports Node/Element native types".into());
        }
        if interfaces.iter().any(|i: &Interface| i.name == name) {
            return Err("duplicate interface".into());
        }
        let parent = interface.inheritance.map(|i| i.identifier.0.to_owned());
        if parent
            .as_ref()
            .is_some_and(|p| !interfaces.iter().any(|i| &i.name == p))
        {
            return Err("parent must be declared before child".into());
        }
        interfaces.push(Interface {
            name,
            parent,
            members,
        });
    }
    Ok(interfaces)
}

fn snake(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_ascii_uppercase() {
                vec!['_', c.to_ascii_lowercase()]
            } else {
                vec![c]
            }
        })
        .collect()
}

pub fn generate(interfaces: &[Interface]) -> (String, String, String) {
    let mut common = String::from("// Generated from idl/dom.webidl.\n");
    let mut boa = common.clone();
    let mut v8 = common.clone();
    let mut boa_install = String::from("fn install(context: &mut Context) -> JsResult<()> {\n");
    let mut v8_install = String::from("fn install(scope: &mut v8::PinScope<'_, '_>) {\n");
    for interface in interfaces {
        let parent = interface
            .parent
            .as_deref()
            .map_or("None".to_owned(), |p| format!("Some(Interface::{p})"));
        boa_install.push_str(&format!(
            "install_interface(context, Interface::{}, {parent})?;\n",
            interface.name
        ));
        v8_install.push_str(&format!(
            "install_interface(scope, Interface::{}, {parent});\n",
            interface.name
        ));
        for member in &interface.members {
            let native = snake(&member.name);
            let callback = format!("{}_{}", snake(&interface.name), native)
                .trim_start_matches('_')
                .to_owned();
            common.push_str(&format!("pub(crate) fn {callback}<C: BindingCall>(call: &mut C) -> Result<C::Value, C::Error> {{\nlet receiver = call.receiver(Interface::{})?;\ncall.require({})?;\n", interface.name, member.arguments.len()));
            for (i, arg) in member.arguments.iter().enumerate() {
                let convert = match arg {
                    Ty::String => "dom_string",
                    Ty::NullableNode => "nullable_node",
                    _ => unreachable!(),
                };
                common.push_str(&format!("let a{i} = call.{convert}({i})?;\n"));
            }
            let args = (0..member.arguments.len())
                .map(|i| format!(", a{i}"))
                .collect::<String>();
            let binding = if member.result == Ty::Undefined {
                ""
            } else {
                "let value = "
            };
            common.push_str(&format!("{binding}call.with_dom(|dom| dom.{native}(receiver{args})).map_err(|e| call.native_error(e))?;\n"));
            let wrap = match member.result {
                Ty::U16 => "Number",
                Ty::NullableNode => "Node",
                Ty::NullableString => "String",
                Ty::Bool => "Boolean",
                Ty::Undefined => "Undefined",
                Ty::String => panic!("non-nullable string return unsupported"),
            };
            let value = if member.result == Ty::Undefined {
                "()"
            } else {
                "value"
            };
            common.push_str(&format!("call.output(NativeValue::{wrap}({value}))\n}}\n"));
            boa.push_str(&format!("fn {callback}(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {{ crate::generated::{callback}(&mut Call {{ this, args, context }}) }}\n"));
            v8.push_str(&format!("fn {callback}<'s>(scope: &mut v8::PinScope<'s, '_>, args: v8::FunctionCallbackArguments<'s>, mut rv: v8::ReturnValue<'s>) {{\nlet result = crate::generated::{callback}(&mut Call {{ scope, args }});\nmatch result {{ Ok(value) => rv.set(value), Err(error) => throw(scope, error) }}\n}}\n"));
            let length = member.arguments.len();
            boa_install.push_str(&format!(
                "install_member(context, Interface::{}, {:?}, {}, {length}, {callback})?;\n",
                interface.name, member.name, member.getter
            ));
            // Keep each callback as a function item: rusty_v8 requires a zero-sized callback type.
            v8_install.push_str(&format!("let function = v8::Function::builder({callback}).length({length}).constructor_behavior(v8::ConstructorBehavior::Throw).build(scope).expect(\"binding function\");\ninstall_member(scope, Interface::{}, {:?}, {}, function);\n", interface.name, member.name, member.getter));
        }
    }
    boa_install.push_str("Ok(())\n}\n");
    v8_install.push_str("}\n");
    boa.push_str(&boa_install);
    v8.push_str(&v8_install);
    (common, boa, v8)
}

#[cfg(test)]
#[path = "codegen/tests.rs"]
mod tests;
