use std::io::{Read, Cursor, Seek, SeekFrom, Write};
use std::fs::{OpenOptions, File};
use std::collections::HashMap;
use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;
use crc32c::*;

use blockchain::{block::*, index::*};

const MAX_CACHE_SIZE: u64 = 100;

pub struct BlockStore {
    indexer: Arc<Indexer>,

    height: u64,
    blocks: HashMap<u64, Arc<SignedBlock>>,
    genesis_block: Option<Arc<SignedBlock>>,

    file: RefCell<File>,
    byte_pos_tail: u64
}

impl BlockStore {

    pub fn new(path: &Path, indexer: Arc<Indexer>) -> BlockStore {
        let (file, tail) = {
            let f = OpenOptions::new()
                                .create(true)
                                .read(true)
                                .append(true)
                                .open(path).unwrap();
            let m = f.metadata().unwrap();
            (f, m.len())
        };

        let height = indexer.get_chain_height();
        let mut store = BlockStore {
            indexer,

            height,
            blocks: HashMap::new(),
            genesis_block: None,

            file: RefCell::new(file),
            byte_pos_tail: tail
        };
        store.genesis_block = store.get(0);

        { // Initialize the cache
            let min = if height < MAX_CACHE_SIZE { height } else { height - 100 };
            let max = min + MAX_CACHE_SIZE;
            for height in min..=max {
                if let Some(block) = store.get_from_disk(height) {
                    store.blocks.insert(height, Arc::new(block));
                } else {
                    break;
                }
            }
        }

        store
    }

    #[inline(always)]
    pub fn get_chain_height(&self) -> u64 {
        self.height
    }

    pub fn get(&self, height: u64) -> Option<Arc<SignedBlock>> {
        if height > self.height {
            return None
        } else if height == 0 {
            if let Some(ref block) = self.genesis_block {
                return Some(Arc::clone(block))
            }
        }
        if let Some(block) = self.blocks.get(&height) {
            Some(Arc::clone(block))
        } else {
            Some(Arc::new(self.get_from_disk(height)?))
        }
    }

    fn get_from_disk(&self, height: u64) -> Option<SignedBlock> {
        if height > self.height { return None }

        let pos = self.indexer.get_block_byte_pos(height)?;
        let mut f = self.file.borrow_mut();
        f.seek(SeekFrom::Start(pos)).unwrap();

        let (block_len, crc) = {
            let mut meta = [0u8; 8];
            f.read_exact(&mut meta).unwrap();
            let len = u32_from_buf!(meta, 0) as usize;
            let crc = u32_from_buf!(meta, 4);
            (len, crc)
        };

        let block_vec = {
            let mut buf = Vec::with_capacity(block_len);
            unsafe { buf.set_len(block_len); }
            f.read_exact(&mut buf).unwrap();
            assert_eq!(crc, crc32c(&buf));
            buf
        };

        let mut cursor = Cursor::<&[u8]>::new(&block_vec);
        let block = SignedBlock::decode_with_tx(&mut cursor).unwrap();
        Some(block)
    }

    pub fn insert(&mut self, block: SignedBlock) {
        assert_eq!(self.height + 1, block.height, "invalid block height");
        let byte_pos = self.byte_pos_tail;
        self.write_to_disk(&block);

        { // Update internal cache
            let height = block.height;
            self.height = height;
            self.indexer.set_block_byte_pos(height, byte_pos);
            self.indexer.set_chain_height(height);

            let opt = self.blocks.insert(height, Arc::new(block));
            if self.blocks.len() > MAX_CACHE_SIZE as usize {
                let b = self.blocks.remove(&(height - MAX_CACHE_SIZE));
                debug_assert!(b.is_some(), "nothing removed from cache");
            }
            debug_assert!(opt.is_none(), "block already in the chain");
        }
    }

    pub fn insert_genesis(&mut self, block: SignedBlock) {
        assert_eq!(block.height, 0, "expected to be 0");
        assert!(self.genesis_block.is_none(), "expected genesis block to not exist");
        self.write_to_disk(&block);
        self.genesis_block = Some(Arc::new(block));
        self.indexer.set_block_byte_pos(0, 0);
    }

    fn write_to_disk(&mut self, block: &SignedBlock) {
        let vec = &mut Vec::with_capacity(1_048_576);
        block.encode_with_tx(vec);
        let len = vec.len() as u32;
        let crc = crc32c(vec);

        let mut f = self.file.borrow_mut();
        {
            let mut buf = [0u8; 8];
            buf[0] = (len >> 24) as u8;
            buf[1] = (len >> 16) as u8;
            buf[2] = (len >> 8) as u8;
            buf[3] = len as u8;

            buf[4] = (crc >> 24) as u8;
            buf[5] = (crc >> 16) as u8;
            buf[6] = (crc >> 8) as u8;
            buf[7] = crc as u8;

            f.write_all(&buf).unwrap();
        }

        f.write_all(vec).unwrap();
        f.flush().unwrap();

        self.byte_pos_tail += u64::from(len) + 8;
    }
}