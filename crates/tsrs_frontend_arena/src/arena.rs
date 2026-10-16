use std::alloc::Layout;
use std::cell::{Cell, RefCell, UnsafeCell};
use std::mem::MaybeUninit;
use std::ptr::NonNull;

const CHUNK: usize = 64 * 1024;

#[repr(C, align(16))]
struct Block(UnsafeCell<MaybeUninit<[u8; 16]>>);

struct Chunk {
    blocks: Box<[Block]>,
    used: usize,
}

/// Storage for one published file version. References borrow this owner; dropping it frees all chunks.
///
/// Allocation is single-threaded. The immutable nodes it contains can be read on scoped worker threads.
/// No thread-local allocator, per-node reference count, or registered global source-text slot is involved.
#[derive(Default)]
pub struct FileArena {
    chunks: RefCell<Vec<Chunk>>,
    used: Cell<usize>,
}

impl FileArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocated_bytes(&self) -> usize {
        self.chunks.borrow().iter().map(|c| c.blocks.len() * 16).sum()
    }

    pub fn used_bytes(&self) -> usize {
        self.used.get()
    }

    pub(crate) fn reserve(&self, layout: Layout) -> NonNull<u8> {
        assert!(layout.align() <= 16, "front-end arena alignment exceeds 16");
        if layout.size() == 0 {
            // No zero-sized allocation is dereferenced; its aligned address is used for empty slices.
            return NonNull::<Block>::dangling().cast();
        }
        let mut chunks = self.chunks.borrow_mut();
        let fits = |c: &Chunk| {
            let start = c.used.next_multiple_of(layout.align());
            start.checked_add(layout.size()).is_some_and(|end| end <= c.blocks.len() * 16)
        };
        if chunks.last().is_none_or(|c| !fits(c)) {
            let count = layout.size().max(CHUNK).div_ceil(16);
            // Each Block is valid with uninitialized contents. Its UnsafeCell permits allocation into
            // disjoint byte ranges while references to previously published allocations remain live.
            let blocks = std::iter::repeat_with(|| Block(UnsafeCell::new(MaybeUninit::uninit()))).take(count).collect();
            chunks.push(Chunk { blocks, used: 0 });
        }
        let chunk = chunks.last_mut().unwrap();
        let start = chunk.used.next_multiple_of(layout.align());
        chunk.used = start + layout.size();
        self.used.set(self.used.get() + layout.size());
        // SAFETY: the box is stable, Block has alignment 16, and fits/reservation bounds cover the
        // fresh range. Mutation is confined to UnsafeCell storage and never overlaps an older value.
        unsafe { NonNull::new_unchecked(chunk.blocks.as_ptr().cast::<u8>().cast_mut().add(start)) }
    }

    pub(crate) fn slot<T>(&self) -> NonNull<T> {
        assert!(!std::mem::needs_drop::<T>(), "published records must not own destructors");
        self.reserve(Layout::new::<T>()).cast()
    }

    pub(crate) fn alloc<T: Copy>(&self, value: T) -> &T {
        let slot = self.slot::<T>();
        // SAFETY: slot is fresh and properly aligned. T has no destructor, and the returned reference
        // cannot outlive this arena. Nothing writes this slot after initialization.
        unsafe {
            slot.as_ptr().write(value);
            slot.as_ref()
        }
    }

    pub(crate) fn slice<T: Copy>(&self, values: &[T]) -> &[T] {
        let slot = self.reserve(Layout::array::<T>(values.len()).expect("arena slice overflow")).cast::<T>();
        // SAFETY: fresh storage for exactly values.len() copies of T, initialized before making the
        // shared slice; the allocation lives until the borrowed arena is dropped.
        unsafe {
            std::ptr::copy_nonoverlapping(values.as_ptr(), slot.as_ptr(), values.len());
            std::slice::from_raw_parts(slot.as_ptr(), values.len())
        }
    }

    pub(crate) fn text(&self, text: &str) -> &str {
        let bytes = self.slice(text.as_bytes());
        std::str::from_utf8(bytes).expect("copied a valid str")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::RawNodeData;
    use crate::model::{Node, RawNode};
    use tsrs_ast::{Kind, NodeFlags};

    #[test]
    fn native_cycles_and_disjoint_growth() {
        let arena = FileArena::new();
        let first = arena.slot::<Node<'_>>();
        let second = arena.slot::<Node<'_>>();
        let flow = arena.slot::<crate::FlowNode<'_>>();
        let payload = arena.alloc(crate::generated::RawIdentifier {
            text: crate::Text(arena.text("café").as_bytes()),
            flow_node: Some(flow),
            lifetime: std::marker::PhantomData,
        });
        let raw = |parent| RawNode {
            kind: Kind::Identifier,
            loc: tsrs_core::TextRange::new(0, 0),
            flags: NodeFlags::None,
            parent: Some(parent),
            data: RawNodeData::Identifier(NonNull::from(payload).cast()),
            jsdoc: arena.slice(&[second]),
        };
        // SAFETY: both slots are fresh, the repr(C) pairs have asserted matching field layouts, and
        // references are created only after both objects (and therefore the entire cycle) are valid.
        let node = unsafe {
            first.cast::<RawNode<'_>>().as_ptr().write(raw(second));
            second.cast::<RawNode<'_>>().as_ptr().write(raw(first));
            flow.cast::<crate::model::RawFlowNode<'_>>().as_ptr().write(crate::model::RawFlowNode {
                flags: tsrs_ast::flow::FlowFlags::Start,
                node: Some(first),
                antecedent: Some(flow),
                antecedents: None,
            });
            first.as_ref()
        };
        let text = arena.text("café");
        let values = arena.slice(&[7u64, 11, 13]);
        let mut blocks = Vec::new();
        for i in 0..8u64 {
            blocks.push(arena.slice(&[i; 2048]));
        }
        assert!(std::ptr::eq(node.parent.unwrap().parent.unwrap(), node));
        assert_eq!(text, "café");
        assert_eq!(node.text().as_str(), Ok("café"));
        assert!(std::ptr::eq(node.jsdoc[0], node.parent.unwrap()));
        assert!(std::ptr::eq(node.flow_node().unwrap().node.unwrap(), node));
        assert!(std::ptr::eq(node.flow_node().unwrap().antecedent.unwrap(), node.flow_node().unwrap()));
        assert_eq!(values, &[7, 11, 13]);
        assert!(blocks.iter().enumerate().all(|(i, b)| b.iter().all(|&v| v == i as u64)));
        assert!(arena.allocated_bytes() >= CHUNK * 2);
        std::thread::scope(|scope| {
            scope.spawn(|| assert_eq!(node.parent.unwrap().kind, Kind::Identifier)).join().unwrap();
        });
    }
}
