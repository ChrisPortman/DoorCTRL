#[derive(Clone)]
pub enum LockState {
    Locked,
    Unlocked,
}

#[derive(Clone)]
pub enum DoorState {
    Open,
    Closed,
}

#[derive(Clone)]
pub enum AnyState {
    LockState(LockState),
    DoorState(DoorState),
}
