mod datetime_parse;
mod scheduler;

use scheduler::*;

use serde::{Serialize, Deserialize};

fn main() {
    let event = Event {
        author: String::from("author"),
        channel: String::from("channel"),
        event: String::from("event"),
        members: vec![String::from("a"), String::from("b")],
        time: String::from("1/1/20 7:03 PM EDT")
    };

    let mut scheduler: Scheduler<Task> = Scheduler::create("redis://127.0.0.1/", None, None).unwrap();

    let job_id = scheduler.schedule_job(&Task::Event(event), 1).unwrap();
    let event: Task = scheduler.get_job(&job_id).unwrap();
    event.call();
    println!("{:?}", event);
}

#[derive(Debug, Deserialize, Serialize)]
enum Task {
    Event(Event)
}

#[derive(Debug, Deserialize, Serialize)]
struct Event {
    pub author: String,
    pub channel: String,
    pub event: String,
    pub members: Vec<String>,
    pub time: String
}

impl Callable for Event {
    fn call(&self) {
        println!("{}, {}, {}, {:?}, {}",
            self.author, self.channel, self.event, self.members, self.time);
    }
}

impl Callable for Task {
    fn call(&self) {
        match self {
            Task::Event(item) => item.call()
        };
    }
}
