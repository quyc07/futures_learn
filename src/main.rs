use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Receiver, sync_channel, SyncSender};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, SystemTime};
use futures::future::BoxFuture;
use futures::FutureExt;
use futures::task::ArcWake;

// 用于异步休眠的Future
pub struct TimeFuture {
    shared_state: Arc<Mutex<SharedState>>,
}

struct SharedState {
    completed: bool,
    waker: Option<Waker>,
}

impl Future for TimeFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared_state = self.shared_state.lock().unwrap();
        if shared_state.completed {
            println!("TimeFuture::poll() -> Poll::Ready(())");
            Poll::Ready(())
        } else {
            shared_state.waker = Some(cx.waker().clone());
            println!("TimeFuture::poll() -> Poll::Pending");
            Poll::Pending
        }
    }
}

impl TimeFuture {
    pub fn new(duration: Duration) -> Self {
        let shared_state = Arc::new(Mutex::new(SharedState {
            completed: false,
            waker: None,
        }));
        let thread_shared_state = shared_state.clone();
        thread::spawn(move || {
            thread::sleep(duration);
            let mut shared_state = thread_shared_state.lock().unwrap();
            shared_state.completed = true;
            if let Some(waker) = shared_state.waker.take() {
                println!("TimeFuture::wake()");
                waker.wake();
            }
            println!("TimeFuture::wake() done");
        });
        TimeFuture { shared_state }
    }
}

struct Executor {
    ready_queue: Receiver<Arc<Task>>,
}

struct Spawner {
    task_sender: SyncSender<Arc<Task>>,
}

struct Task {
    future: Mutex<Option<BoxFuture<'static, ()>>>,
    task_sender: SyncSender<Arc<Task>>,
}

fn new_executor_and_spawner() -> (Executor, Spawner) {
    const MAX_QUEUED_TASKS: usize = 10_000;
    let (task_sender, ready_queue) = sync_channel(MAX_QUEUED_TASKS);
    (Executor { ready_queue }, Spawner { task_sender })
}

impl Spawner {
    fn spawn(&self, future: impl Future<Output=()> + 'static + Send) {
        let future = future.boxed();
        let task = Arc::new(Task {
            future: Mutex::new(Some(future)),
            task_sender: self.task_sender.clone(),
        });
        self.task_sender.send(task).expect("too many tasks queued");
    }
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let cloned = arc_self.clone();
        arc_self.task_sender.send(cloned).expect("too many tasks queued");
    }
}

fn check_if_print() -> bool {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_micros()
        % 1000000
        == 1
}

impl Executor {
    fn run(&self) {
        // loop {
        //     match self.ready_queue.recv() {
        //         Ok(task) => {
        //             let mut future_slot = task.future.lock().unwrap();
        //             if let Some(mut future) = future_slot.take() {
        //                 let waker = futures::task::waker_ref(&task);
        //                 let context = &mut Context::from_waker(&*waker);
        //                 if let Poll::Pending = future.as_mut().poll(context) {
        //                     *future_slot = Some(future);
        //                 }
        //             }
        //         }
        //         Err(e) => {
        //             if check_if_print() {
        //                 println!("{}", e.to_string());
        //             }
        //         }
        //     };
        // }

        while let Ok(task) = self.ready_queue.recv() {
            let mut future_slot = task.future.lock().unwrap();
            if let Some(mut future) = future_slot.take() {
                let waker = futures::task::waker_ref(&task);
                let context = &mut Context::from_waker(&*waker);
                if let Poll::Pending = future.as_mut().poll(context) {
                    *future_slot = Some(future);
                }
            }
        }
    }
}

fn main() {
    let (executor, spawner) = new_executor_and_spawner();
    spawner.spawn(async {
        println!("howdy!");
        TimeFuture::new(Duration::from_secs(2)).await;
        println!("done!");
    });
    spawner.spawn(async {
        println!("hello!");
        TimeFuture::new(Duration::from_secs(1)).await;
        println!("world!");
    });
    drop(spawner);
    executor.run();
}