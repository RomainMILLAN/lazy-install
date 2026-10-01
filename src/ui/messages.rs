use crate::session::Fact;

/// What a modal input produced.
#[derive(Debug)]
pub enum Action {
    None,
    InputSubmit(String),
    InputCancel,
}

/// What the background threads send to the loop.
#[derive(Debug)]
pub enum BgMsg {
    /// Forwarded as is to `Session::apply`.
    Fact(Fact),
    /// A run printed something: redraw.
    PtyOutputReceived,
}
