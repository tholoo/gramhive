use bon::bon;
use regex::Regex;
use std::{
    any::Any,
    collections::HashMap,
    pin::Pin,
    sync::{Arc, OnceLock, RwLock},
};

use crate::{GenericResult, router::Router};

#[derive(Debug, Clone)]
pub enum ArgumentKind {
    Any,
    Text(Regex),
    Media,
    Document,
}

#[derive(Debug, Clone)]
pub struct Argument {
    pub name: &'static str,
    pub optional: bool,
    pub try_from_reply: bool,
    pub kind: ArgumentKind,
}

#[derive(Debug, Clone)]
pub struct InputArgument<T> {
    pub name: &'static str,
    pub value: T,
}

impl<T> InputArgument<T> {
    pub fn new(name: &'static str, value: T) -> Self {
        Self { name, value }
    }
}

#[derive(Debug, Clone)]
pub enum Constraint {
    Present(&'static str),
    And(Box<Constraint>, Box<Constraint>),
    Or(Box<Constraint>, Box<Constraint>),
    /// XOR: exactly one of the two must be provided.
    Xor(&'static str, &'static str),
}

impl Constraint {
    pub fn and(self, other: Constraint) -> Self {
        Constraint::And(Box::new(self), Box::new(other))
    }

    pub fn or(self, other: Constraint) -> Self {
        Constraint::Or(Box::new(self), Box::new(other))
    }
}

static PREFIXES: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

fn init_prefixes() -> &'static RwLock<Vec<String>> {
    PREFIXES.get_or_init(|| RwLock::new(vec!["/".to_string()]))
}

pub type WrapFnOutput = Pin<Box<dyn Future<Output = GenericResult> + Send + 'static>>;
pub type WrapFn = fn(dptree::di::DependencyMap) -> WrapFnOutput;

#[derive(Debug, Clone)]
pub struct CommandMeta {
    pub extras: HashMap<String, Arc<dyn Any + Send + Sync>>,
    pub cmds: &'static [&'static str],
    pub description: Option<&'static str>,
    pub module: Option<&'static str>,
    pub sig: Option<&'static str>,
    pub regex: Regex,
}

#[bon]
impl CommandMeta {
    #[builder]
    pub fn new(
        #[builder(field)] extras: HashMap<String, Arc<dyn Any + Send + Sync>>,
        cmds: &'static [&'static str],
        description: Option<&'static str>,
        module: Option<&'static str>,
        sig: Option<&'static str>,
    ) -> Self {
        let prefix_pattern = CommandMeta::get_prefixes()
            .iter()
            .map(|s| regex::escape(s))
            .collect::<Vec<_>>()
            .join("|");
        let cmd_pattern = cmds.join("|");
        let regex = Regex::new(&format!(
            r#"^(?P<prefix>{})(?P<cmd>{})(?:\s|$)(?P<input>.*)"#,
            prefix_pattern, cmd_pattern
        ))
        .unwrap();

        CommandMeta {
            extras,
            cmds,
            description,
            module,
            sig,
            regex,
        }
    }

    pub(crate) fn reinit_regex(&mut self, bot_username: &str) -> &mut Self {
        let prefix_pattern = CommandMeta::get_prefixes()
            .iter()
            .map(|s| regex::escape(s))
            .collect::<Vec<_>>()
            .join("|");
        let cmd_pattern = self.cmds.join("|");
        let regex = Regex::new(&format!(
            r#"^(?P<prefix>{})(?P<cmd>{})(?:\@{})?(?:\s|$)(?P<input>.*)"#,
            prefix_pattern, cmd_pattern, bot_username
        ))
        .unwrap();
        self.regex = regex;
        self
    }
}

impl<S: command_meta_builder::State> CommandMetaBuilder<S> {
    pub fn extra<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Any + Send + Sync + 'static,
    {
        self.extras.insert(key.into(), Arc::new(value));
        self
    }
}

impl CommandMeta {
    pub fn extra<T: Any>(&self, key: &str) -> Option<&T> {
        self.extras.get(key)?.downcast_ref::<T>()
    }

    pub fn register(self, router: &mut Router) -> Self {
        router.add_command(self.clone());
        self
    }

    pub fn get_prefixes() -> Vec<String> {
        init_prefixes().read().unwrap().clone()
    }

    pub fn set_prefixes(new_prefixes: Vec<String>) {
        *init_prefixes().write().unwrap() = new_prefixes;
    }
}

#[derive(Debug, Clone)]
pub struct CommandInput {
    pub prefix: String,
    pub cmd: String,
    pub input: Option<String>,
    pub args: Vec<String>,
    pub meta: CommandMeta,
}
