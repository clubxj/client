use std::time::Duration;

#[derive(Clone, Default)]
pub struct Wake {
    sender: Option<async_channel::Sender<Option<Duration>>>,
}

impl Wake {
    pub fn channel() -> (Self, async_channel::Receiver<Option<Duration>>) {
        let (sender, receiver) = async_channel::unbounded();
        (
            Self {
                sender: Some(sender),
            },
            receiver,
        )
    }

    pub fn wake(&self) {
        if let Some(sender) = &self.sender {
            sender.try_send(None).ok();
        }
    }

    pub fn wake_after(&self, delay: Duration) {
        if let Some(sender) = &self.sender {
            sender.try_send(Some(delay)).ok();
        }
    }
}
