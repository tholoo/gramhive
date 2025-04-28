use dptree::{Handler, di::DependencyMap};
use grammers_client::{Update, types::Message};

use crate::{
    GenericResult,
    commands::{CommandInput, CommandMeta},
};

pub type DpResult = dptree::Handler<'static, DependencyMap, GenericResult>;

pub trait HandlerExt<Output> {
    #[must_use]
    fn filter_command(self, command: CommandMeta) -> Self;
}

#[must_use]
pub fn filter_command<Output>(command: CommandMeta) -> Handler<'static, DependencyMap, Output>
where
    Output: Send + Sync + 'static,
{
    dptree::filter_map(move |message: Message| {
        if let Some(caps) = command.regex.captures(message.text()) {
            let prefix = caps.name("prefix")?.as_str().to_string();
            let cmd = caps.name("cmd")?.as_str().to_string();

            let input = match caps.name("input") {
                Some(input) if !input.is_empty() => Some(input.as_str().to_string()),
                _ => None,
            };

            let args: Vec<String> = input
                .as_deref()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect();

            Some(CommandInput {
                prefix,
                cmd,
                input,
                args,
                meta: command.clone(),
            })
        } else {
            None
        }
    })
}

impl<Output> HandlerExt<Output> for Handler<'static, DependencyMap, Output>
where
    Output: Send + Sync + 'static,
{
    fn filter_command(self, command: CommandMeta) -> Self {
        self.chain(filter_command::<Output>(command))
    }
}

mod private {
    use grammers_client::{Update, types::Message};

    pub trait Sealed {}

    impl Sealed for Update {}
    impl Sealed for Message {}
}

macro_rules! define_ext {
    ($ext_name:ident, $for_ty:ty => $( ($func:ident, $proj_fn:expr, $fn_doc:expr $(, $map:ident)? ) ,)*) => {
        #[doc = concat!("Filter methods for [`", stringify!($for_ty), "`].")]
        pub trait $ext_name<Out>: private::Sealed {
            $( define_ext!(@sig $func, $fn_doc); )*
        }

        impl<Out> $ext_name<Out> for $for_ty
        where
            Out: Send + Sync + 'static,
        {
            $( define_ext!(@impl $for_ty, $func, $proj_fn $(, $map )? ); )*
        }
    };

    (@sig $func:ident, $fn_doc:expr) => {
        #[doc = $fn_doc]
        fn $func() -> Handler<'static, DependencyMap, Out>;
    };

    (@impl $for_ty:ty, $func:ident, $proj_fn:expr, $map:ident) => {
        fn $func() -> Handler<'static, DependencyMap, Out> {
            dptree::filter_map(move |input: $for_ty| {
                $proj_fn(input)
            })
        }
    };

    (@impl $for_ty:ty, $func:ident, $proj_fn:expr) => {
        fn $func() -> Handler<'static, DependencyMap, Out> {
            dptree::filter(move |input: $for_ty| {
                $proj_fn(input)
            })
        }
    };
}

macro_rules! define_update_ext {
    ($( ($func:ident, $kind:path) ,)*) => {
        define_ext! {
            UpdateFilterExt, Update =>
            $(
                (
                    $func,
                    |update: Update| match update {
                        $kind(x) => Some(x),
                        _ => None,
                    },
                    concat!("Filters out [`", stringify!($kind), "`] objects."),
                    map
                ),
            )*
        }
    }
}

define_update_ext! {
    (filter_new_message, Update::NewMessage),
    (filter_edited_message, Update::MessageEdited),
    (filter_deleted_message, Update::MessageDeleted),
    (filter_callback_query, Update::CallbackQuery),
    (filter_inline_query, Update::InlineQuery),
    (filter_inline_send, Update::InlineSend),
    (filter_inline_raw, Update::Raw),
}

macro_rules! define_message_ext {
    ($( ($func:ident, $body:expr),)*) => {
        define_ext! {
            MessageFilterExt, Message =>
            $(
                (
                    $func,
                    $body,
                    concat!("Applies the [`", stringify!($func), "`] filter.")
                ),
            )*
        }
    };
}

define_message_ext! {
    (filter_text, |message: Message| !message.text().is_empty()),
    (filter_outgoing, |message: Message| message.outgoing()),
    (filter_incoming, |message: Message| !message.outgoing()),
    (filter_mentioned, |message: Message| message.mentioned()),
    (filter_media_unread, |message: Message| message.media_unread()),
    (filter_silent, |message: Message| message.silent()),
    (filter_post, |message: Message| message.post()),
    (filter_from_scheduled, |message: Message| message.from_scheduled()),
    (filter_edit_hidden, |message: Message| message.edit_hide()),
    (filter_pinned, |message: Message| message.pinned()),
    (filter_has_sender, |message: Message| message.sender().is_some()),
    (filter_has_forward, |message: Message| message.forward_header().is_some()),
    (filter_has_reply, |message: Message| message.reply_header().is_some()),
    (filter_has_markup, |message: Message| message.reply_markup().is_some()),
    (filter_has_entities, |message: Message| message.fmt_entities().is_some()),
    (filter_has_media, |message: Message| message.media().is_some()),
    (filter_has_photo, |message: Message| message.photo().is_some()),
    (filter_has_reactions, |message: Message| message.reaction_count().unwrap_or(0) > 0),
    (filter_edited, |message: Message| message.edit_date().is_some()),
    (filter_has_post_author, |message: Message| message.post_author().is_some()),
    (filter_grouped, |message: Message| message.grouped_id().is_some()),
    (filter_restricted, |message: Message| message.restriction_reason().is_some()),
    (filter_service, |message: Message| message.action().is_some()),
    (filter_reply_to_message_id, |message: Message| message.reply_to_message_id().is_some()),
    (filter_viewed, |message: Message| message.view_count().unwrap_or(0) > 0),
    (filter_forwarded, |message: Message| message.forward_count().unwrap_or(0) > 0),
    (filter_has_replies, |message: Message| message.reply_count().unwrap_or(0) > 0),
    (filter_via_bot, |message: Message| message.via_bot_id().is_some()),
}
