pub struct DependencyQueue {
    pub queue: Vec<String>,
    pub in_progress: Vec<String>,
    pub completed: Vec<String>,
}
