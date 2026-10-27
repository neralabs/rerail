//! A compact value arena has no unowned prefix or aliased mutable meanings.
use super::*;
use metal_regex_frontier::{
    RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork,
};

#[derive(Clone, Copy)]
enum Fault {
    Control,
    ScratchPrefix,
    WorkStatusAlias,
}

fn exercise(fault: Fault) {
    for fault in [Fault::Control, fault] {
        metal::objc::rc::autoreleasepool(|| {
            let pattern = "^# a$";
            let program = Program::new(
                vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
                vec![TextPredicate::LuaPattern {
                    source: 1,
                    compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                }],
                vec![0],
                vec![],
            )
            .unwrap();
            let value::ValueProgram::Lua(lua) = &program.value_programs[0] else {
                unreachable!()
            };
            let (matched, cost) = lua.test_work_count(b"# a");
            assert_eq!(matched, Ok(true));
            assert_eq!(cost, 4);
            let resident = metal_grid_plan::ResidentInput::new(
                metal_grid::MetalGridInput::build(&[program], &[tape("# a")], &[vec![vec![0]]])
                    .unwrap(),
                None,
            )
            .unwrap();
            let guard = metal_backend::metal_execution_guard().unwrap();
            let device = metal::Device::system_default().unwrap();
            let queue = device.new_command_queue();
            let data = metal_register_execution::upload(&device, &resident).unwrap();
            let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
            let words = metal_register_execution::read::<u32>(
                &code.code,
                code.code.buffer.length() as usize / 4,
            )
            .unwrap();
            let value = resident.input.packed.programs[0].value_start;
            let width = words[4 + value as usize * 4 + 1];
            let prefix = u32::from(matches!(fault, Fault::ScratchPrefix));
            let request = RegexRequest {
                value,
                source_start: resident.input.packed.files[0].source_start,
                source_length: 3,
                budget: MAX_STEPS_PER_PATH as u32,
                scratch: prefix,
                width,
                match_start: u32::MAX,
                match_end: u32::MAX,
                ..RegexRequest::default()
            };
            let requests = metal_backend::MappedBuffer::from_slice(&device, &[request]).unwrap();
            let ids = metal_backend::MappedBuffer::from_slice(&device, &[0u32]).unwrap();
            let status = metal_backend::MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
            let work = metal_backend::MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
            let failures = metal_backend::MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
            let input_status = if matches!(fault, Fault::WorkStatusAlias) {
                &work
            } else {
                &status
            };
            let actual = code.execute(
                &device,
                &queue,
                RegexInput {
                    data: &data,
                    status: input_status,
                    budget_owners: 1,
                },
                RegexBatch {
                    scratch_plan: ScratchPlan::Device,
                    requests,
                    ids,
                    count: 1,
                    scratch_slots: (width + prefix) as usize,
                },
                SharedWork {
                    values: &work,
                    failures: &failures,
                    evaluations: 1,
                    abort_evaluation: false,
                },
            );
            let charged = metal_register_execution::read::<u32>(&work, 1).unwrap();
            let rows = actual
                .as_ref()
                .ok()
                .map(|buffer| metal_register_execution::read::<RegexRequest>(buffer, 1));
            let failure_codes = metal_register_execution::read::<u32>(&failures, 1);
            drop(guard);
            if matches!(fault, Fault::Control) {
                let rows = rows.unwrap().unwrap();
                assert_eq!(rows[0].status, 2);
                assert_eq!(rows[0].used, cost as u32);
                assert_eq!(charged, [cost as u32]);
            } else {
                assert!(
                    matches!(actual, Err(Error::BackendFailure)),
                    "accepted={}, error={:?}",
                    actual.is_ok(),
                    actual.as_ref().err()
                );
                assert_eq!(
                    charged,
                    [0],
                    "invalid inputs must preserve the caller's work counter"
                );
            }
            assert_eq!(failure_codes.unwrap(), [0]);
        });
    }
}

#[test]
fn value_request_scratch_intervals_reject_unowned_prefixes() {
    exercise(Fault::ScratchPrefix);
}

#[test]
fn value_request_mutable_work_cannot_alias_status() {
    exercise(Fault::WorkStatusAlias);
}
