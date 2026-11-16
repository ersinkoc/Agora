import json, os, pathlib, signal, subprocess, time, shutil, sys, fcntl
BINARY=pathlib.Path(sys.argv[1]).resolve()
SHIM=pathlib.Path(sys.argv[2]).resolve()
root=pathlib.Path('/tmp')/f'a551-{os.getpid()}'
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
 wait_for(lambda:(root/'gate/ready').exists(),'down reached selected unlink under its guard')
 old_lock=lock.read_text()
 marker=root/'gate/claim-ready'
 waiting=dict(base,LD_PRELOAD=str(SHIM),AGORA_REVIEW_LOCK=str(lock),AGORA_REVIEW_FLOCK_READY=str(marker))
 supervisor=subprocess.Popen([str(BINARY),'supervise','node'],env=waiting,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 wait_for(marker.exists,'replacement claimant reached flock')
 # Confirm a real kernel-held exclusive guard, rather than relying on the replacement being slow.
 with open(str(lock)+'.guard','r+b') as probe:
  blocked=False
  try:fcntl.flock(probe,fcntl.LOCK_EX|fcntl.LOCK_NB)
  except BlockingIOError:blocked=True
  assert blocked,'down must hold the exclusive OS guard while paused before unlink'
 time.sleep(.1)
 assert supervisor.poll() is None,'waiting replacement exited'
 assert lock.read_text()==old_lock,'replacement modified the lock while removal held the guard'
 report['replacement_reached_os_lock']=True
 report['guard_exclusively_held_before_unlink']=True
 report['role_lock_unchanged_while_guard_held']=True
 report['replacement_pid']=supervisor.pid
 (root/'gate/release').touch()
 out,err=down.communicate(timeout=15)
 report['first_down']={'exit':down.returncode,'stdout':out.strip(),'stderr':err.strip()}
 assert down.returncode==0
 wait_for(lambda:lock_pid()==supervisor.pid,'replacement claims after removal releases guard')
 def child_ready():
  try:
   ids=[int(x) for x in pathlib.Path(f'/proc/{supervisor.pid}/task/{supervisor.pid}/children').read_text().split()]
   return [pid for pid in ids if live(pid)]
  except FileNotFoundError:return []
 child_pids=wait_for(child_ready,'replacement starts node after guard release')
 children=[(pid,info(pid)[1]) for pid in child_pids]
 report['replacement_lock_after_release']=lock.read_text().strip()
 report['replacement_children']=child_pids
 report['replacement_alive_with_visible_lock']=supervisor.poll() is None and lock_pid()==supervisor.pid
 assert report['replacement_alive_with_visible_lock']
 second=subprocess.run([str(BINARY),'down'],env=base,capture_output=True,text=True,timeout=15)
 report['second_down']={'exit':second.returncode,'stdout':second.stdout.strip(),'stderr':second.stderr.strip()}
 assert second.returncode==0 and 'Stopped the node' in second.stdout
 supervisor.wait(timeout=5)
 wait_for(lambda:not any(live(pid,birth) for pid,birth in children),'second down stops node child',5)
 report['supervisor_stopped_by_second_down']=True
 report['node_stopped_by_second_down']=True
 report['role_lock_absent_after_second_down']=not lock.exists()
 assert report['role_lock_absent_after_second_down']
 print(json.dumps(report,indent=2),flush=True)
 pathlib.Path('/tmp/agora-551-guard-proof.json').write_text(json.dumps(report,indent=2)+'\n')
finally:
 (root/'gate/release').touch()
 if down and down.poll() is None:down.kill();down.wait()
 if supervisor and supervisor.poll() is None:
  supervisor.terminate()
  try:supervisor.wait(timeout=3)
  except subprocess.TimeoutExpired:supervisor.kill();supervisor.wait()
 for pid,birth in children:
  if live(pid,birth):os.kill(pid,signal.SIGKILL)
 wait_for(lambda:not any(live(pid,birth) for pid,birth in children),'test children stopped',3)
 shutil.rmtree(root)
