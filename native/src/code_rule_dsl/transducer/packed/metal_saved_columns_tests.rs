//! Independent bit-by-bit transport truth includes every u32 width and partial tiles.
use super::*;
use crate::code_rule_dsl::transducer::packed::metal_backend::{metal_execution_guard, pipeline_named};

const DECODE: &str = r#"
kernel void test_saved_columns_decode(
 device const uint* payload [[buffer(0)]],device const uint2* heads [[buffer(1)]],
 device uint* output [[buffer(2)]],device atomic_uint* status [[buffer(3)]],
 constant uint4& p [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 uint arena=0u,local=i,fields=0u,header_start=0u;
 for(;arena<4u;arena++){
  fields=arena==0u?10u:(arena==1u?9u:(arena==2u?8u:4u));
  uint cells=p.x*fields;if(local<cells)break;local-=cells;header_start+=((p.x+255u)/256u)*fields;
 }
 if(arena==4u)return;
 ColumnView v=literal_columns(payload,heads,p.x,fields,header_start,p.z,p.y,1u,status);
 output[i]=column_value(v,local/fields,local%fields);
}
"#;

#[test]
fn gpu_column_transport_preserves_all_bits_and_rejects_inconsistent_headers() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    // This crosses the physical-format threshold and the scan's first level.
    // Its final tile has 37 rows; field widths vary across every full tile.
    let rows = 280_613_usize;
    let widths = [10, 9, 8, 4];
    let mut sources = Vec::new();
    let mut expected_headers = Vec::new();
    let mut expected_words = Vec::new();
    let mut seen_widths = [false; 33];
    for (arena, fields) in widths.into_iter().enumerate() {
        let mut values = vec![0_u32; rows * fields];
        for row in 0..rows {
            for field in 0..fields {
                let bits = (row / 256 * 31 + field + arena * 7) % 33;
                let mask = u32::MAX.checked_shr(32 - bits as u32).unwrap_or(0);
                let base = u32::MAX - mask;
                let delta = match row % 256 {
                    0 => 0,
                    1 => mask,
                    _ => {
                        (row as u32)
                            .wrapping_mul(0x9e37_79b9)
                            .rotate_left(field as u32)
                            & mask
                    }
                };
                values[row * fields + field] = base + delta;
            }
        }
        // This reference writes individual bits. It does not share the GPU's
        // word-packing algorithm, prefix scan, or header selection logic.
        for first in (0..rows).step_by(256) {
            let count = (rows - first).min(256);
            for field in 0..fields {
                let column = (first..first + count).map(|row| values[row * fields + field]);
                let minimum = column.clone().min().unwrap();
                let maximum = column.max().unwrap();
                let bits = 32 - (maximum - minimum).leading_zeros();
                seen_widths[bits as usize] = true;
                let offset = expected_words.len();
                expected_headers.push([
                    minimum,
                    (offset as u32 * 4) | (bits & 31) | ((bits >> 5) << 31),
                ]);
                expected_words.resize(offset + (count * bits as usize).div_ceil(256) * 8, 0_u32);
                for row in 0..count {
                    let delta = values[(first + row) * fields + field] - minimum;
                    for bit in 0..bits as usize {
                        if delta & (1 << bit) != 0 {
                            let target = row * bits as usize + bit;
                            expected_words[offset + target / 32] |= 1 << (target % 32);
                        }
                    }
                }
            }
        }
        sources.push(values);
    }
    assert!(seen_widths.into_iter().all(|seen| seen));
    let arenas = OutputArenas {
        rows: MappedBuffer::from_slice(&device, &sources[0]).unwrap(),
        captures: MappedBuffer::from_slice(&device, &sources[1]).unwrap(),
        properties: MappedBuffer::from_slice(&device, &sources[2]).unwrap(),
        bytes: MappedBuffer::zeroed::<u8>(&device, 0).unwrap(),
        layout: Some(MappedBuffer::from_slice(&device, &sources[3]).unwrap()),
        counts: [rows as u32, rows as u32, rows as u32, 0],
    };
    let encoded = save(&device, &queue, arenas).unwrap();
    let SavedArenas::Encoded {
        headers,
        payload,
        words,
        groups,
        ..
    } = &encoded
    else {
        panic!("this control must exercise the production encoded format");
    };
    assert!(*groups > 4096);
    assert_eq!(*groups as usize, expected_headers.len());
    assert_eq!(*words as usize, expected_words.len());
    for (index, (actual, expected)) in read::<[u32; 2]>(headers, expected_headers.len())
        .unwrap()
        .into_iter()
        .zip(&expected_headers)
        .enumerate()
    {
        assert_eq!(actual, *expected, "encoded header {index}");
    }
    for (index, (actual, expected)) in read::<u32>(payload, expected_words.len())
        .unwrap()
        .into_iter()
        .zip(&expected_words)
        .enumerate()
    {
        assert_eq!(actual, *expected, "encoded payload word {index}");
    }
    let cells = sources.iter().map(Vec::len).sum::<usize>();
    let output = MappedBuffer::zeroed::<u32>(&device, cells).unwrap();
    let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
    let source = format!("{}\n{DECODE}", super::super::SOURCE);
    let pipeline = pipeline_named(&source, "test_saved_columns_decode").unwrap();
    let decode = || {
        metal::objc::rc::autoreleasepool(|| {
            let command = queue.new_command_buffer();
            encode(
                command,
                &pipeline,
                "test_saved_columns_decode",
                &[
                    &payload.buffer,
                    &headers.buffer,
                    &output.buffer,
                    &status.buffer,
                ],
                &[rows as u32, *words, *groups, 0],
                cells.div_ceil(256),
                None,
            );
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
        });
        read::<u32>(&status, 1).unwrap()[0]
    };
    assert_eq!(decode(), 0);
    for (index, (actual, expected)) in read::<u32>(&output, cells)
        .unwrap()
        .into_iter()
        .zip(sources.into_iter().flatten())
        .enumerate()
    {
        assert_eq!(actual, expected, "decoded scalar {index}");
    }
    for malformed in 0..4 {
        unsafe {
            std::ptr::copy_nonoverlapping(
                expected_headers.as_ptr(),
                headers.buffer.contents().cast(),
                expected_headers.len(),
            );
            status.buffer.contents().cast::<u32>().write(0);
            let h = std::slice::from_raw_parts_mut(
                headers.buffer.contents().cast::<[u32; 2]>(),
                expected_headers.len(),
            );
            match malformed {
                0 => h[1][1] = (h[1][1] & 0x7fff_ffe0) | 0x8000_0001, // width 33
                1 => h[1][1] = 0x7fff_ffe1,                           // outside the actual payload
                2 => h[1][0] = u32::MAX, // nonzero delta overflows its literal base
                3 => h[2][1] &= 0x8000_001f, // alias another column's payload
                _ => unreachable!(),
            }
        }
        assert_eq!(decode(), 2, "malformed header {malformed}");
    }
}
