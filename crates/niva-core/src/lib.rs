//! Niva assistant core: pure logic with no I/O.

pub mod ports;
pub mod state_machine;

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert_eq!(2 + 2, 4);
    }
}
