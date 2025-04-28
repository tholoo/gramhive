# GramHive

GramHive is a high-level library on top of [grammers](https://github.com/Lonami/grammers).

# features
- Multi-client support
- Dptree handlers (similar to [teloxide](https://github.com/teloxide/teloxide))
- Useful abstractions to make development easier

# Example

```rust
use gramhive::*;

#[tokio::main]
async fn main() {
  let client = ClientBuilder::bot("123:abc")
      .api_id(1234)
      .api_hash("abcd")
      .session("my_client.session")
      .catch_up(false)
      .connect()
      .await
      .unwrap()
      .authorize()
      .await
      .unwrap();

  let mut router = Router::new();
  router.set_error_handler(dptree::endpoint(move |error: ArcBoxedError| async move {
      error!("dispatch error: {}", error);
  }));
  let router = Arc::new(router);

  let handler = Update::filter_new_message().branch(
      dptree::entry()
          .filter_command(
              CommandMeta::builder()
                  .cmds(&["start"])
                  .description("Start the bot")
                  .build()
                  .register(router),
          )
          .endpoint(start),
  );
  router.add(handler);

  let mut swarm = Swarm::new();
  swarm.add(
      SwarmObject::new(Arc::new(client), router.clone(), deps![
          router.clone()
      ])
      .await?,
  );

  swarm.run().await.unwrap();
}

async fn start(message: Message) -> GenericResult {
    message.reply(tg_html().bold("Started!")).await?;
}
```

