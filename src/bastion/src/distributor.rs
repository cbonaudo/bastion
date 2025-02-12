//! `Distributor` is a mechanism that allows you to send messages to children.

use crate::{
    message::{Answer, Message, MessageHandler},
    prelude::{ChildRef, SendError},
    system::{STRING_INTERNER, SYSTEM},
};
use anyhow::Result as AnyResult;
use futures::{channel::oneshot, FutureExt};
use futures_timer::Delay;
use lasso::Spur;
use std::{
    fmt::Debug,
    sync::mpsc::{channel, Receiver},
    time::Duration,
};

// Copy is fine here because we're working
// with interned strings here
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The `Distributor` is the main message passing mechanism we will use.
/// it provides methods that will allow us to send messages
/// and add/remove actors to the Distribution list
pub struct Distributor(Spur);

impl Distributor {
    /// Create a new distributor to send messages to
    /// # Example
    ///
    /// ```rust
    /// # use bastion::prelude::*;
    /// #
    /// #
    /// # fn run() {
    /// #
    /// let distributor = Distributor::named("my target group");
    /// // distributor is now ready to use
    /// # }
    /// ```
    pub fn named(name: impl AsRef<str>) -> Self {
        Self(STRING_INTERNER.get_or_intern(name.as_ref()))
    }

    /// Ask a question to a recipient attached to the `Distributor`
    /// and wait for a reply.
    ///
    /// This can be achieved manually using a `MessageHandler` and `ask_one`.
    /// Ask a question to a recipient attached to the `Distributor`
    /// # Example
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # async fn run() {
    /// # Bastion::init();
    /// # Bastion::start();
    ///
    /// # Bastion::supervisor(|supervisor| {
    /// #    supervisor.children(|children| {
    /// // attach a named distributor to the children
    /// children
    /// #        .with_redundancy(1)
    ///     .with_distributor(Distributor::named("my distributor"))
    ///     .with_exec(|ctx: BastionContext| {
    ///        async move {
    ///            loop {
    ///                // The message handler needs an `on_question` section
    ///                // that matches the `question` you're going to send,
    ///                // and that will reply with the Type the request expects.
    ///                // In our example, we ask a `&str` question, and expect a `bool` reply.                    
    ///                MessageHandler::new(ctx.recv().await?)
    ///                    .on_question(|message: &str, sender| {
    ///                        if message == "is it raining today?" {
    ///                            sender.reply(true).unwrap();
    ///                        }
    ///                    });
    ///            }
    ///            Ok(())
    ///        }
    ///     })
    /// #   })
    /// # });
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// let reply: Result<String, SendError> = distributor
    ///     .request("is it raining today?")
    ///     .await
    ///     .expect("couldn't receive reply");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn request<R: Message>(
        &self,
        question: impl Message,
    ) -> oneshot::Receiver<Result<R, SendError>> {
        let (sender, receiver) = oneshot::channel();
        let s = *self;
        spawn!(async move {
            match SYSTEM.dispatcher().ask(s, question) {
                Ok(response) => match response.await {
                    Ok(message) => {
                        let message_to_send = MessageHandler::new(message)
                            .on_tell(|reply: R, _| Ok(reply))
                            .on_fallback(|_, _| {
                                Err(SendError::Other(anyhow::anyhow!(
                                    "received a message with the wrong type"
                                )))
                            });
                        let _ = sender.send(message_to_send);
                    }
                    Err(e) => {
                        let _ = sender.send(Err(SendError::Other(anyhow::anyhow!(
                            "couldn't receive reply: {:?}",
                            e
                        ))));
                    }
                },
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
            };
        });

        receiver
    }

    /// Ask a question to a recipient attached to the `Distributor`
    /// and wait for a reply.
    ///
    /// this is the sync variant of the `request` function, backed by a futures::channel::oneshot
    /// # Example
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// #
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # fn run() {
    /// # Bastion::init();
    /// # Bastion::start();
    /// # Bastion::supervisor(|supervisor| {
    /// #    supervisor.children(|children| {
    /// // attach a named distributor to the children
    /// children
    /// #        .with_redundancy(1)
    ///     .with_distributor(Distributor::named("my distributor"))
    ///     .with_exec(|ctx: BastionContext| {
    ///        async move {
    ///            loop {
    ///                // The message handler needs an `on_question` section
    ///                // that matches the `question` you're going to send,
    ///                // and that will reply with the Type the request expects.
    ///                // In our example, we ask a `&str` question, and expect a `bool` reply.                    
    ///                MessageHandler::new(ctx.recv().await?)
    ///                    .on_question(|message: &str, sender| {
    ///                        if message == "is it raining today?" {
    ///                            sender.reply(true).unwrap();
    ///                        }
    ///                    });
    ///            }
    ///            Ok(())
    ///        }
    ///     })
    /// #   })
    /// # });
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// let reply: Result<bool, SendError> = distributor
    ///    .request_sync("is it raining today?")
    ///    .recv()
    ///    .expect("couldn't receive reply"); // Ok(true)
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn request_sync<R: Message>(
        &self,
        question: impl Message,
    ) -> Receiver<Result<R, SendError>> {
        let (sender, receiver) = channel();
        let s = *self;
        spawn!(async move {
            match SYSTEM.dispatcher().ask(s, question) {
                Ok(response) => {
                    if let Ok(message) = response.await {
                        let message_to_send = MessageHandler::new(message)
                            .on_tell(|reply: R, _| Ok(reply))
                            .on_fallback(|_, _| {
                                Err(SendError::Other(anyhow::anyhow!(
                                    "received a message with the wrong type"
                                )))
                            });
                        let _ = sender.send(message_to_send);
                    } else {
                        let _ = sender.send(Err(SendError::Other(anyhow::anyhow!(
                            "couldn't receive reply"
                        ))));
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
            };
        });

        receiver
    }

    /// Ask a question to a recipient attached to the `Distributor`
    /// and wait for a reply.
    ///
    /// This is the timeout variant of the 'request' function. If the request cannot be performed
    /// in the provided duration, an Error is propagated.
    /// # Example
    ///
    /// ```no_run
    /// # use core::time::Duration;
    /// # use bastion::prelude::*;
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # async fn run() {
    /// # Bastion::init();
    /// # Bastion::start();
    ///
    /// # Bastion::supervisor(|supervisor| {
    /// #    supervisor.children(|children| {
    /// // attach a named distributor to the children
    /// children
    /// #        .with_redundancy(1)
    ///     .with_distributor(Distributor::named("my distributor"))
    ///     .with_exec(|ctx: BastionContext| {
    ///        async move {
    ///            loop {
    ///                // The message handler needs an `on_question` section
    ///                // that matches the `question` you're going to send,
    ///                // and that will reply with the Type the request expects.
    ///                // In our example, we ask a `&str` question, and expect a `bool` reply.                    
    ///                MessageHandler::new(ctx.recv().await?)
    ///                    .on_question(|message: &str, sender| {
    ///                        if message == "is it raining today?" {
    ///                            sender.reply(true).unwrap();
    ///                        }
    ///                    });
    ///            }
    ///            Ok(())
    ///        }
    ///     })
    /// #   })
    /// # });
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// let timeout = Duration::from_millis(10);
    /// let reply: Result<String, SendError> = distributor
    ///     .request_timeout("is it raining today?", timeout)
    ///     .await
    ///     .expect("couldn't receive reply");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn request_timeout<R: Message>(
        &self,
        question: impl Message,
        timeout: Duration,
    ) -> oneshot::Receiver<Result<R, SendError>> {
        let (sender, receiver) = oneshot::channel();
        let s = *self;
        spawn!(async move {
            match SYSTEM.dispatcher().ask(s, question) {
                Ok(response) => {
                    futures::select! {
                        response_awaited = response.fuse() => {
                            match response_awaited {
                                Ok(message) => {
                                    let message_to_send = MessageHandler::new(message)
                                        .on_tell(|reply: R, _| Ok(reply))
                                        .on_fallback(|_, _| {
                                            Err(SendError::Other(anyhow::anyhow!(
                                                "received a message with the wrong type"
                                            )))
                                        });
                                    let _ = sender.send(message_to_send);
                                }
                                Err(e) => {
                                    let _ = sender.send(Err(SendError::Other(anyhow::anyhow!(
                                        "couldn't receive reply: {:?}",
                                        e
                                    ))));
                                }
                            }
                        },
                        _duration = Delay::new(timeout).fuse() => {
                            let _ = sender.send(Err(SendError::Other(anyhow::anyhow!(
                                "operation timed out before finish"
                            ))));
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
            }
        });

        receiver
    }

    /// Ask a question to a recipient attached to the `Distributor`
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// #
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # fn run() {
    /// # Bastion::init();
    /// # Bastion::supervisor(|supervisor| {
    /// #    supervisor.children(|children| {
    ///     children
    ///         .with_redundancy(1)
    ///         .with_distributor(Distributor::named("my distributor"))
    ///         .with_exec(|ctx: BastionContext| { // ...
    /// #           async move {
    /// #               loop {
    /// #                   let _: Option<SignedMessage> = ctx.try_recv().await;
    /// #               }
    /// #               Ok(())
    /// #           }
    ///         })
    /// #    })
    /// # });
    /// #
    /// # Bastion::start();
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// let answer: Answer = distributor.ask_one("hello?").expect("couldn't send question");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn ask_one(&self, question: impl Message) -> Result<Answer, SendError> {
        SYSTEM.dispatcher().ask(*self, question)
    }

    /// Ask a question to all recipients attached to the `Distributor`
    ///
    /// Requires a `Message` that implements `Clone`. (it will be cloned and passed to each recipient)
    /// # Example
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// #
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # fn run() {
    /// # Bastion::init();
    /// # Bastion::supervisor(|supervisor| {
    /// #    supervisor.children(|children| {
    /// #    children
    /// #        .with_redundancy(1)
    /// #        .with_distributor(Distributor::named("my distributor"))
    /// #        .with_exec(|ctx: BastionContext| {
    /// #           async move {
    /// #               loop {
    /// #                   let _: Option<SignedMessage> = ctx.try_recv().await;
    /// #               }
    /// #               Ok(())
    /// #           }
    /// #        })
    /// #    })
    /// # });
    /// #
    /// # Bastion::start();
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// let answer: Vec<Answer> = distributor.ask_everyone("hello?".to_string()).expect("couldn't send question");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn ask_everyone(&self, question: impl Message + Clone) -> Result<Vec<Answer>, SendError> {
        SYSTEM.dispatcher().ask_everyone(*self, question)
    }

    /// Send a Message to a recipient attached to the `Distributor`
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// #
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # fn run() {
    /// # Bastion::init();
    /// # Bastion::supervisor(|supervisor| {
    /// #    supervisor.children(|children| {
    /// #    children
    /// #        .with_redundancy(1)
    /// #        .with_distributor(Distributor::named("my distributor"))
    /// #        .with_exec(|ctx: BastionContext| {
    /// #           async move {
    /// #               loop {
    /// #                   let _: Option<SignedMessage> = ctx.try_recv().await;
    /// #               }
    /// #               Ok(())
    /// #           }
    /// #        })
    /// #    })
    /// # });
    /// #
    /// # Bastion::start();
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// let answer: () = distributor.tell_one("hello?").expect("couldn't send question");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn tell_one(&self, message: impl Message) -> Result<(), SendError> {
        SYSTEM.dispatcher().tell(*self, message)
    }

    /// Send a Message to each recipient attached to the `Distributor`
    ///
    /// Requires a `Message` that implements `Clone`. (it will be cloned and passed to each recipient)
    /// # Example
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// #
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # fn run() {
    /// # Bastion::init();
    /// # Bastion::supervisor(|supervisor| {
    /// #    supervisor.children(|children| {
    /// #    children
    /// #        .with_redundancy(1)
    /// #        .with_distributor(Distributor::named("my distributor"))
    /// #        .with_exec(|ctx: BastionContext| {
    /// #           async move {
    /// #               loop {
    /// #                   let _: Option<SignedMessage> = ctx.try_recv().await;
    /// #               }
    /// #               Ok(())
    /// #           }
    /// #        })
    /// #    })
    /// # });
    /// #
    /// # Bastion::start();
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// let answer: () = distributor.tell_one("hello?").expect("couldn't send question");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn tell_everyone(&self, message: impl Message + Clone) -> Result<Vec<()>, SendError> {
        SYSTEM.dispatcher().tell_everyone(*self, message)
    }

    /// subscribe a `ChildRef` to the named `Distributor`
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// #
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # fn run() {
    /// # Bastion::init();
    /// # let children =
    /// #    Bastion::children(|children| {
    /// #    children
    /// #        .with_redundancy(1)
    /// #        .with_distributor(Distributor::named("my distributor"))
    /// #        .with_exec(|ctx: BastionContext| {
    /// #           async move {
    /// #               loop {
    /// #                   let _: Option<SignedMessage> = ctx.try_recv().await;
    /// #               }
    /// #               Ok(())
    /// #           }
    /// #        })
    /// # }).unwrap();
    /// #
    /// # Bastion::start();
    /// #
    /// let child_ref = children.elems()[0].clone();
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// // child_ref will now be elligible to receive messages dispatched through distributor
    /// distributor.subscribe(child_ref).expect("couldn't subscribe child to distributor");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn subscribe(&self, child_ref: ChildRef) -> AnyResult<()> {
        SYSTEM.dispatcher().register_recipient(self, child_ref)
    }

    /// unsubscribe a `ChildRef` to the named `Distributor`
    ///
    /// ```no_run
    /// # use bastion::prelude::*;
    /// #
    /// # #[cfg(feature = "tokio-runtime")]
    /// # #[tokio::main]
    /// # async fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # #[cfg(not(feature = "tokio-runtime"))]
    /// # fn main() {
    /// #    run();    
    /// # }
    /// #
    /// # fn run() {
    /// # Bastion::init();
    /// # let children =
    /// #    Bastion::children(|children| {
    /// #    children
    /// #        .with_redundancy(1)
    /// #        .with_distributor(Distributor::named("my distributor"))
    /// #        .with_exec(|ctx: BastionContext| {
    /// #           async move {
    /// #               loop {
    /// #                   let _: Option<SignedMessage> = ctx.try_recv().await;
    /// #               }
    /// #               Ok(())
    /// #           }
    /// #        })
    /// # }).unwrap();
    /// #
    /// # Bastion::start();
    /// #
    /// let child_ref = children.elems()[0].clone();
    ///
    /// let distributor = Distributor::named("my distributor");
    ///
    /// // child_ref will not receive messages dispatched through the distributor anymore
    /// distributor.unsubscribe(child_ref).expect("couldn't unsubscribe child to distributor");
    ///
    /// # Bastion::stop();
    /// # Bastion::block_until_stopped();
    /// # }
    /// ```
    pub fn unsubscribe(&self, child_ref: ChildRef) -> AnyResult<()> {
        let global_dispatcher = SYSTEM.dispatcher();
        global_dispatcher.remove_recipient(&vec![*self], child_ref)
    }

    pub(crate) fn interned(&self) -> &Spur {
        &self.0
    }
}

#[cfg(test)]
mod distributor_tests {
    use crate::prelude::*;
    use core::time;
    use futures::channel::mpsc::channel;
    use futures::{SinkExt, StreamExt};
    use std::{thread, time::Duration};

    const TEST_DISTRIBUTOR: &str = "test distributor";
    const SUBSCRIBE_TEST_DISTRIBUTOR: &str = "subscribe test";

    #[cfg(feature = "tokio-runtime")]
    #[tokio::test]
    async fn test_tokio_distributor() {
        blocking!({
            run_tests();
        });
    }

    #[cfg(not(feature = "tokio-runtime"))]
    #[test]
    fn distributor_tests() {
        run_tests();
    }

    fn run_tests() {
        setup();

        test_tell();
        test_ask();
        test_request();
        test_subscribe();
    }

    fn test_subscribe() {
        let temp_distributor = Distributor::named("temp distributor");

        assert!(
            temp_distributor.tell_one("hello!").is_err(),
            "should not be able to send message to an empty distributor"
        );

        let one_child: ChildRef = run!(async {
            Distributor::named(SUBSCRIBE_TEST_DISTRIBUTOR)
                .request(())
                .await
                .unwrap()
                .unwrap()
        });
        temp_distributor.subscribe(one_child.clone()).unwrap();

        temp_distributor
            .tell_one("hello!")
            .expect("should be able to send message a distributor that has a subscriber");

        temp_distributor.unsubscribe(one_child).unwrap();

        assert!(
            temp_distributor.tell_one("hello!").is_err(),
            "should not be able to send message to a distributor who's sole subscriber unsubscribed"
        );
    }

    fn test_tell() {
        let test_distributor = Distributor::named(TEST_DISTRIBUTOR);

        test_distributor
            .tell_one("don't panic and carry a towel")
            .unwrap();

        let sent = test_distributor
            .tell_everyone("so long, and thanks for all the fish")
            .unwrap();

        assert_eq!(
            5,
            sent.len(),
            "test distributor is supposed to have 5 children"
        );
    }

    fn test_ask() {
        let test_distributor = Distributor::named(TEST_DISTRIBUTOR);

        let question: String =
            "What is the answer to life, the universe and everything?".to_string();

        run!(async {
            let answer = test_distributor.ask_one(question.clone()).unwrap();
            MessageHandler::new(answer.await.unwrap())
                .on_tell(|answer: u8, _| {
                    assert_eq!(42, answer);
                })
                .on_fallback(|unknown, _sender_addr| {
                    panic!("unknown message\n {:?}", unknown);
                });
        });

        run!(async {
            let answers = test_distributor.ask_everyone(question.clone()).unwrap();
            assert_eq!(
                5,
                answers.len(),
                "test distributor is supposed to have 5 children"
            );
            let meanings = futures::future::join_all(answers.into_iter().map(|answer| async {
                MessageHandler::new(answer.await.unwrap())
                    .on_tell(|answer: u8, _| {
                        assert_eq!(42, answer);
                        answer
                    })
                    .on_fallback(|unknown, _sender_addr| {
                        panic!("unknown message\n {:?}", unknown);
                    })
            }))
            .await;

            assert_eq!(
                42 * 5,
                meanings.iter().sum::<u8>(),
                "5 children returning 42 should sum to 42 * 5"
            );
        });
    }

    fn test_request() {
        let test_distributor = Distributor::named(TEST_DISTRIBUTOR);

        let question: String =
            "What is the answer to life, the universe and everything?".to_string();

        run!(async {
            let answer: u8 = test_distributor
                .request(question.clone())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(42, answer);
        });

        let answer_sync: u8 = test_distributor
            .request_sync(question.clone())
            .recv()
            .unwrap()
            .unwrap();

        assert_eq!(42, answer_sync);

        run!(async {
            let timeout = Duration::from_millis(100);
            let answer_timeout: u8 = test_distributor
                .request_timeout(question.clone(), timeout)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(42, answer_timeout);
        });

        run!(async {
            let timeout = Duration::from_nanos(1);
            let answer_timeout: Result<u8, SendError> = test_distributor
                .request_timeout(question.clone(), timeout)
                .await
                .unwrap();

            let err_msg: SendError = answer_timeout.unwrap_err();
            assert!(matches!(err_msg, SendError::Other { .. }));
        });
    }

    fn setup() {
        Bastion::init();
        Bastion::start();

        const NUM_CHILDREN: usize = 5;

        // This channel and the use of callbacks will allow us to know when all of the children are spawned.
        let (sender, receiver) = channel(NUM_CHILDREN);

        Bastion::supervisor(|supervisor| {
            let test_ready = sender.clone();
            let subscribe_test_ready = sender.clone();
            supervisor
                .children(|children| {
                    children
                        .with_redundancy(NUM_CHILDREN)
                        .with_distributor(Distributor::named(TEST_DISTRIBUTOR))
                        .with_callbacks(Callbacks::new().with_after_start(move || {
                            let mut test_ready = test_ready.clone();
                            spawn!(async move { test_ready.send(()).await });
                        }))
                        .with_exec(|ctx| async move {
                            loop {
                                let child_ref = ctx.current().clone();
                                MessageHandler::new(ctx.recv().await?)
                                    .on_question(|_: String, sender| {
                                        thread::sleep(time::Duration::from_millis(10));
                                        let _ = sender.reply(42_u8);
                                    })
                                    // send your child ref
                                    .on_question(|_: (), sender| {
                                        let _ = sender.reply(child_ref);
                                    });
                            }
                        })
                    // Subscribe / unsubscribe tests
                })
                .children(|children| {
                    children
                        .with_distributor(Distributor::named(SUBSCRIBE_TEST_DISTRIBUTOR))
                        .with_callbacks(Callbacks::new().with_after_start(move || {
                            let mut subscribe_test_ready = subscribe_test_ready.clone();
                            spawn!(async move { subscribe_test_ready.send(()).await });
                        }))
                        .with_exec(|ctx| async move {
                            loop {
                                let child_ref = ctx.current().clone();
                                MessageHandler::new(ctx.recv().await?).on_question(
                                    |_: (), sender| {
                                        let _ = sender.reply(child_ref);
                                    },
                                );
                            }
                        })
                })
        })
        .unwrap();

        // Wait until the children have spawned
        run!(async {
            // NUM_CHILDREN for the test distributor group,
            // 1 for the subscribe test group
            receiver.take(NUM_CHILDREN + 1).collect::<Vec<_>>().await;
        });
    }
}
