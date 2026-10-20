"""Real child processes only; no provider, guest or network access."""
import ast
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

script = json.load(sys.stdin)
tree = ast.parse(script)
namespace = {}
exec(compile(ast.Module(body=[node for node in tree.body if not isinstance(node, ast.Try)],
                        type_ignores=[]), '<guest-definitions>', 'exec'), namespace)
run_mc = namespace['run_mc']
ReconcileError = namespace['ReconcileError']
marker = 'PRIVATE-CREDENTIAL-MUST-NOT-ESCAPE'


def invoke(code, timeout=2):
    return run_mc([sys.executable, '-c', code], os.environ.copy(), timeout)


def refused(code, error, timeout=2):
    start = time.monotonic()
    try:
        invoke(code, timeout)
    except ReconcileError as caught:
        assert str(caught) == error
        assert marker not in repr(caught)
    else:
        raise AssertionError('unbounded child accepted')
    assert time.monotonic() - start < timeout + 2


result = invoke("import os; os.write(1,b'{\"status\":\"success\"}'); os.write(2,b'private warning')")
assert result.returncode == 0 and json.loads(result.stdout) == {'status': 'success'}
assert result.stderr == b''  # never retain provider diagnostics
result = invoke('import sys; sys.exit(9)')
assert result.returncode == 9
# Each stream and their aggregate must be bounded while the child is still alive.
for streams in ((1,), (2,), (1, 2)):
    code = f'''import os,signal,time
signal.signal(signal.SIGTERM,signal.SIG_IGN)
for _ in range(100):
 for fd in {streams!r}: os.write(fd,({marker!r}.encode()*3000)[:65536])
time.sleep(10)
'''
    refused(code, 'InvalidProviderResponse')
# Neither stream alone exceeds the limit; their sum must still be rejected.
refused('import os; os.write(1,b"x"*600000); os.write(2,b"x"*600000)',
        'InvalidProviderResponse')
refused('import time; time.sleep(10)', 'ProviderTimeout', .1)
# EOF on both pipes must not remove the deadline on process exit.
refused('import os,time; os.close(1); os.close(2); time.sleep(10)', 'ProviderTimeout', .1)

with tempfile.TemporaryDirectory(prefix='s3-process-test-') as temporary:
    root = Path(temporary)
    for mode in ('timeout', 'overflow'):
        pid_path = root / (mode + '.pid')
        lock_path = root / (mode + '.lock')
        child_code = ('import os,signal,time,pathlib,fcntl; '
                      f'lock=open({str(root / (mode + ".lock"))!r},"w"); '
                      'fcntl.flock(lock,fcntl.LOCK_EX); '
                      'signal.signal(signal.SIGTERM,signal.SIG_IGN); '
                      f'pathlib.Path({str(pid_path)!r}).write_text(str(os.getpid())); '
                      'time.sleep(10)')
        code = f'''import os,subprocess,sys,time,pathlib
child=subprocess.Popen([sys.executable,'-c',{child_code!r}])
while not pathlib.Path({str(pid_path)!r}).exists(): time.sleep(.005)
'''
        if mode == 'overflow':
            code += 'while True: os.write(2,b"x"*65536)\n'
        else:
            # Parent exits but grandchild retains both pipes and ignores TERM.
            code += 'sys.exit(0)\n'
        refused(code, 'ProviderTimeout' if mode == 'timeout' else 'InvalidProviderResponse', .25)
        pid = int(pid_path.read_text())
        # A killed descendant releases its actual kernel lock even when an init
        # process has not reaped its zombie yet. No host process inventory needed.
        with lock_path.open('a') as held:
            for attempt in range(50):
                try:
                    fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    time.sleep(.02)
            else:
                os.kill(pid, signal.SIGKILL)
                raise AssertionError('child process group still holds its lock')
    # Actual reconciliation entry point, actual malicious mc, secret-free token.
    executable = root / 'mc'
    executable.write_text('#!' + sys.executable + '\nimport os,time\n'
                          'while True: os.write(2,b"PRIVATE-CREDENTIAL-MUST-NOT-ESCAPE"*2000)\n')
    executable.chmod(0o700)
    environment = dict(os.environ, PATH=str(root) + os.pathsep + os.environ['PATH'])
    payload = {'endpoint': 'http://objects:9000', 'adminAccess': 'admin',
               'adminSecret': marker, 'accessKey': 'app', 'secretKey': marker,
               'bucket': 'app-files', 'owner': 'vzctl:demo/files'}
    response = subprocess.run([sys.executable, '-c', script], input=json.dumps(payload),
                              capture_output=True, text=True, env=environment, timeout=4)
    assert response.returncode == 1
    assert json.loads(response.stdout) == {'error': 'InvalidProviderResponse', 'operation': 'bucket.list'}
    assert not response.stderr and marker not in response.stdout
print('real process limits, deadlines, cleanup and redaction passed')
