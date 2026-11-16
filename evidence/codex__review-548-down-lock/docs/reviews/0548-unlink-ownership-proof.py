import json, os, pathlib, signal, subprocess, time, shutil, sys
BINARY=pathlib.Path(sys.argv[1]).resolve()
SHIM=pathlib.Path(sys.argv[2]).resolve()
root=pathlib.Path('/tmp')/f'a548-{os.getpid()}'
root.mkdir();(root/'run').mkdir();(root/'gate').mkdir();(root/'proj').mkdir()
base=dict(os.environ,AGORA_HOME=str(root),AGORA_LANG='en')
# The project and all daemon state are isolated and no coding agent is launched.
git_env=dict(base,GIT_AUTHOR_NAME='review',GIT_AUTHOR_EMAIL='review@example.com',GIT_COMMITTER_NAME='review',GIT_COMMITTER_EMAIL='review@example.com')
for args in [['init','-q','-b','main'],['commit','--allow-empty','-q','-m','fixture']]:
 subprocess.run(['git','-C',str(root/'proj'),*args],env=git_env,check=True)
(root/'machine.toml').write_text(f'roles = ["node"]\nui = "none"\nnode_projects = ["{root}/proj"]\n')
lock=root/'run/node.lock'
# An abandoned lock with a live PID but an impossible birth time is stale; no process is terminated for it.
lock.write_text(f'{os.getpid()} 18446744073709551600 review-stale\n')

def wait_for(check,label,seconds=8):
 deadline=time.monotonic()+seconds
 while time.monotonic()<deadline:
  value=check()
  if value:return value
  time.sleep(.01)
 raise RuntimeError('timeout: '+label)
def info(pid):
 try:
  fields=pathlib.Path(f'/proc/{pid}/stat').read_text().split(') ',1)[1].split()
  return (fields[0],fields[19])
 except FileNotFoundError:return None
def live(pid,birth=None):
 value=info(pid)
 return bool(value and value[0]!='Z' and (birth is None or value[1]==birth))
def lock_pid():
 try:return int(lock.read_text().split()[0])
 except (FileNotFoundError,ValueError,IndexError):return None

down=None;supervisor=None;children=[];report={}
try:
 hooked=dict(base,LD_PRELOAD=str(SHIM),AGORA_REVIEW_LOCK=str(lock),AGORA_REVIEW_GATE=str(root/'gate'))
 down=subprocess.Popen([str(BINARY),'down'],env=hooked,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
 wait_for(lambda:(root/'gate/ready').exists(),'down passed its ownership check and reached unlink')
 supervisor=subprocess.Popen([str(BINARY),'supervise','node'],env=base,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 wait_for(lambda:lock_pid()==supervisor.pid,'replacement supervisor owns lock')
 def child_ready():
  try:
   ids=[int(x) for x in pathlib.Path(f'/proc/{supervisor.pid}/task/{supervisor.pid}/children').read_text().split()]
   return [pid for pid in ids if live(pid)]
  except FileNotFoundError:return []
 child_pids=wait_for(child_ready,'replacement supervisor starts node')
 children=[(pid,info(pid)[1]) for pid in child_pids]
 report['replacement_lock_before_release']=lock.read_text().strip()
 report['replacement_pid']=supervisor.pid
 report['replacement_children']=child_pids
 (root/'gate/release').touch()
 out,err=down.communicate(timeout=15)
 report['first_down']={'exit':down.returncode,'stdout':out.strip(),'stderr':err.strip()}
 time.sleep(.1)
 report['lock_exists_after_first_down']=lock.exists()
 report['supervisor_alive_after_first_down']=supervisor.poll() is None
 report['node_alive_after_first_down']=all(live(pid,birth) for pid,birth in children)
 second=subprocess.run([str(BINARY),'down'],env=base,capture_output=True,text=True,timeout=15)
 report['second_down']={'exit':second.returncode,'stdout':second.stdout.strip(),'stderr':second.stderr.strip()}
 report['supervisor_alive_after_second_down']=supervisor.poll() is None
 report['node_alive_after_second_down']=all(live(pid,birth) for pid,birth in children)
 print(json.dumps(report,indent=2),flush=True)
 pathlib.Path('/tmp/agora-548-unlink-proof.json').write_text(json.dumps(report,indent=2)+'\n')
 assert not(report['supervisor_alive_after_first_down'] and not report['lock_exists_after_first_down']), 'down deleted the replacement supervisor lock after checking the stale holder'
finally:
 (root/'gate/release').touch()
 if down and down.poll() is None:
  down.kill();down.wait()
 # Cleanup is fenced to the exact processes spawned here, never a name-wide kill.
 if supervisor and supervisor.poll() is None:
  supervisor.terminate()
  try:supervisor.wait(timeout=3)
  except subprocess.TimeoutExpired:supervisor.kill();supervisor.wait()
 for pid,birth in children:
  if live(pid,birth):os.kill(pid,signal.SIGKILL)
 wait_for(lambda:not any(live(pid,birth) for pid,birth in children),'test children stopped',3)
 shutil.rmtree(root)
