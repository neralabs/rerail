import hashlib,json,pathlib,shutil,subprocess,tarfile
root=pathlib.Path(__file__).resolve().parent
workspace=pathlib.Path('/Users/jeremy/orca/workspaces/rerail/Async-family-integration-comparison')
out=workspace/'benchmark/async-family-integration/input-lifecycle-production'
manifest=json.loads((root/'manifest.json').read_text())
binary=pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/target/profiling/deps/rerail_engine-9943fa461d0e6845')
manifest['finalTestBinarySha256']=hashlib.file_digest(binary.open('rb'),'sha256').hexdigest()
manifest['rustc']=subprocess.check_output(['/Users/jeremy/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc','--version'],text=True).strip()
(root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
copy=['run.py','axes.py','extra.py','audit.py','package.py','summary.json','runs.json','profile-costs.json','axes-results.json','axes-summary.json','axes-files.json','containment.json','containment-phase.json','validation.json','manifest.json','source-sha256.json','final-source-sha256.json','uncached-process.json']
for name in copy:shutil.copy2(root/name,out/name)
files=set(p for p in root.iterdir() if p.is_file() and p.suffix in ['.json','.log','.txt','.patch','.sha256'])
files.update(p for p in (root/'pre-bulk').rglob('*') if p.is_file())
for profile in root.glob('*/profile'):
    if profile.is_dir(): files.update(p for p in profile.rglob('*') if p.is_file())
with tarfile.open(out/'raw-reports.tar.xz','w:xz',preset=6) as tar:
    for path in sorted(files): tar.add(path,arcname=str(path.relative_to(root)),recursive=False)
print('Archived',len(files),'files,',(out/'raw-reports.tar.xz').stat().st_size,'bytes')
