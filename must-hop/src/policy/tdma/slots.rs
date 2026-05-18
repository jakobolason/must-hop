use core::fmt;

#[cfg(not(feature = "in_std"))]
use defmt::debug;
#[cfg(feature = "in_std")]
use log::debug;

#[derive(Copy, Clone)]
pub struct SlotMask {
    mask: u8,
}
impl Default for SlotMask {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SlotMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Taken Slots: [")?;
        let mut first = true;

        for i in 0..8 {
            if self.is_taken(i) {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}", i)?;
                first = false;
            }
        }
        write!(f, "]")
    }
}

#[cfg(not(feature = "in_std"))]
impl defmt::Format for SlotMask {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "Taken Slots: [");
        let mut first = true;

        for i in 0..8 {
            if self.is_taken(i) {
                if !first {
                    defmt::write!(fmt, ", ");
                }
                // We use {=u8} to tell defmt exactly what type it is sending over the wire
                defmt::write!(fmt, "{=u8}", i);
                first = false;
            }
        }
        defmt::write!(fmt, "]");
    }
}

/// Used to retrieve the mask as u8 in an ergonomic way
impl From<SlotMask> for u8 {
    fn from(msk: SlotMask) -> Self {
        msk.mask
    }
}

impl SlotMask {
    pub const fn new() -> Self {
        Self { mask: 0 }
    }
    fn over_capacity(&self, size: u8) -> bool {
        let bit_capacity = u8::BITS as u8;
        size >= bit_capacity
    }
    /// To set a slot inside mask, does not accept slots higher than 7
    pub fn claim(&mut self, slot: u8) {
        // Do not allow over the bit limit of self.mask(u8 -> 8bits)
        if self.over_capacity(slot) {
            return;
        }
        // shift 1 over to slot pos, and or with mask
        self.mask |= 1 << slot;
    }

    /// Check given slot is occupied. Returns false for slots beyond mask capacity —
    /// those slots are never assigned, so the node correctly sleeps during them.
    pub fn is_taken(&self, slot: u8) -> bool {
        if self.over_capacity(slot) {
            return false;
        }
        (self.mask & (1 << slot)) != 0
    }

    /// Get the next available slot given another node's mask and yours. Uses the node_id to avoid conflicts in race conditions
    pub fn slot_assignment_strat(
        &self,
        max_slots: u8,
        another_mask: u8,
        node_id: u8,
    ) -> Option<u8> {
        let combined_mask = SlotMask {
            mask: self.mask | another_mask,
        };
        // Never assign a slot we can't record in the mask
        let assignable = max_slots.min(u8::BITS as u8);
        let start_offset = node_id % assignable;

        (0..assignable).find_map(|i| {
            let slot = (start_offset + i) % assignable;
            debug!("looking in slot {}", slot);
            if !combined_mask.is_taken(slot) {
                Some(slot)
            } else {
                None
            }
        })
    }
}
