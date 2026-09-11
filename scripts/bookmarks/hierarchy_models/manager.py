"""Apple-only directory hierarchy runtime. No downloads or cloud inference."""
import argparse, json, os, subprocess, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
IDS = ('apple',)
_PARENT_PID = os.getppid()

def default_root():
    # Local development installation; packaged apps use their own application cache.
    repo = HERE.parents[2]
    if (repo/'Cargo.toml').is_file():
        return repo/'.runtime/toc-models'
    return Path.home()/'Library/Caches/org.museion.binarize/toc-models'


def atomic(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name+f'.{os.getpid()}.tmp')
    tmp.write_text(json.dumps(value, ensure_ascii=False, indent=2)); tmp.replace(path)


def check_cancel(cancel):
    if os.getppid() != _PARENT_PID:
        raise InterruptedError('operation_cancelled: application exited')
    if cancel and Path(cancel).exists():
        raise InterruptedError('operation_cancelled')


def apple_paths(root):
    local = Path(root)/'apple'
    folder = local if (local/'check-model').is_file() else HERE/'apple'
    return folder/'check-model', folder/'hierarchy-worker'


def status(root, model):
    if model not in IDS: raise ValueError('Unknown model')
    root = Path(root)
    if model == 'apple':
        check, worker = apple_paths(root)
        value = dict(provider=model, label='Apple', state='unavailable', available=False)
        if not check.is_file():
            value['message'] = 'Apple 检测组件尚未安装'
        else:
            try:
                proc = subprocess.run([str(check)], capture_output=True, text=True, timeout=20)
                raw = json.loads(proc.stdout)
                value['diagnostic'] = raw
                available = raw.get('available') is True or raw.get('availability') == 'available'
                value.update(available=available and worker.is_file(), state='ready' if available and worker.is_file() else 'unavailable',
                             message=str(raw.get('availability', raw.get('reason', raw))))
                if available: value['message'] = '可用 · 本地 Apple 模型' if worker.is_file() else 'Apple 图像推理组件未安装'
                elif 'modelNotReady' in value['message']: value['message'] = '系统模型尚未就绪（modelNotReady）'
                elif 'appleIntelligenceNotEnabled' in value['message']: value['message'] = 'Apple Intelligence 未启用'
                elif 'deviceNotEligible' in value['message']: value['message'] = '此设备不支持 Apple Intelligence'
            except Exception as e: value['message'] = str(e)
        return value


def infer(root, model, request, work, cancel=None, timeout=90):
    if model not in IDS: raise ValueError('Unknown model')
    if request.get('mode') != 'image' or not request.get('images'): raise ValueError('必须直接输入原页图像')
    if not status(root, model)['available']: raise RuntimeError('Apple 模型不可用，请检查系统状态')
    root, work = Path(root), Path(work); work.mkdir(parents=True, exist_ok=True)
    started = time.monotonic(); check_cancel(cancel)
    if model == 'apple':
        with (work/'apple.stdout').open('w') as out, (work/'apple.stderr').open('w') as err:
            child = subprocess.Popen([str(apple_paths(root)[1])], stdin=subprocess.PIPE, stdout=out, stderr=err)
            try:
                child.stdin.write(json.dumps(request).encode()); child.stdin.close()
                while child.poll() is None:
                    check_cancel(cancel)
                    if time.monotonic()-started > timeout: raise TimeoutError('Apple 推理超时')
                    time.sleep(.1)
                result = json.loads((work/'apple.stdout').read_text())
            finally:
                if child.poll() is None: child.kill()
                child.wait()
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['status'])
    parser.add_argument('--root', type=Path, default=default_root())
    args = parser.parse_args()
    print(json.dumps(status(args.root, 'apple'), ensure_ascii=False))

if __name__ == '__main__': main()
