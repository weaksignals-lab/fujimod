//! Persistent DMA bounds for an isolated sequence of guarded calls.
//! Bounds survive routine return so deferred device work remains constrained.
pub struct Policy {
    readable: Vec<(u64,u64)>,
    writable: Vec<(u64,u64)>,
    limit: usize,
    pub requests: Vec<(bool,u64,u64)>,
}
impl Policy {
    pub fn new(readable: Vec<(u64,u64)>, writable: Vec<(u64,u64)>, limit: usize) -> Self {
        Self { readable, writable, limit, requests: Vec::new() }
    }
    pub fn require_same(&self, read: &[(u64,u64)], write: &[(u64,u64)], limit: usize) -> Result<(),String> {
        if read != self.readable || write != self.writable || limit != self.limit {
            return Err("Device-backed caller DMA bounds are frozen for the session".into());
        }
        Ok(())
    }
    pub fn check(&mut self, address: u64, size: usize, write: bool) -> Result<(),String> {
        let ranges = if write { &self.writable } else { &self.readable };
        if size == 0 || self.requests.len() >= self.limit || !address.checked_add(size as u64)
            .is_some_and(|end| ranges.iter().any(|&(lo,hi)| lo <= address && end <= hi)) {
            return Err(format!("Driver DMA guard rejected {} at {address:#x} size={size}", if write { "write" } else { "read" }));
        }
        self.requests.push((write,address,size as u64)); Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_request_limit_are_full_span_and_persistent() {
        let mut p=Policy::new(vec![(0x1000,0x1010)],vec![(0x2000,0x2010)],2);
        assert!(p.check(0x1000,16,false).is_ok());
        assert!(p.check(0x1008,16,false).is_err());
        assert!(p.check(0x1000,4,true).is_err());
        assert!(p.check(u64::MAX,2,true).is_err());
        assert!(p.require_same(&[(0x1000,0x1010)],&[(0x2000,0x2010)],2).is_ok());
        assert!(p.require_same(&[(0x1000,0x1020)],&[(0x2000,0x2010)],2).is_err());
        assert!(p.check(0x2000,16,true).is_ok());
        assert!(p.check(0x2000,1,true).is_err());
        assert_eq!(p.requests.len(),2);
    }
}
