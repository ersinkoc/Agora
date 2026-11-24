#!/usr/bin/env python3
# Seeds a stopped hub database with a large fleet for scripts/perf-tree.mjs: NODES machines x PROJECTS projects x AGENTS agents.
# Standard library only; writes protobuf by hand. Usage: perf-seed-agents.py HUB_DB [nodes projects agents]
import sqlite3, sys, time
def varint(n):
    out=b''
    while True:
        b=n&0x7f; n>>=7
        if n: out+=bytes([b|0x80])
        else: return out+bytes([b])
def f(num, wire, data): return varint((num<<3)|wire)+data
def s(num, text):
    b=text.encode(); return f(num,2,varint(len(b))+b)
def v(num, n): return f(num,0,varint(n))
def msg(num, body): return f(num,2,varint(len(body))+body)
db=sqlite3.connect(sys.argv[1])
now=int(time.time()*1000)
nodes, projects_per, agents_per = (int(x) for x in (sys.argv[2:5] or [10, 5, 10]))
for n in range(nodes):
    node=f'machine-{n:02d}'
    seq=0
    def put(body_field, body):
        global seq
        seq+=1
        ev=s(1,node)+v(2,seq)+v(3,now-60000)+msg(body_field, body)
        db.execute('INSERT INTO events(producer,seq,at_ms,payload) VALUES(?,?,?,?)',(node,seq,now-60000,ev))
    put(10, s(1,node)+s(2,node)+s(3,'linux')+s(4,'0.3.11'))
    for p in range(projects_per):
        pid=f'project-{n*projects_per+p:02d}'
        path=f'/home/dev/code/{pid}'
        put(20, s(1,f'c{p}')+s(2,node)+s(3,pid)+s(4,path))
        # Explorer derives an agent's project through its worktree and checkout.
        put(21, s(1,f'c{p}')+s(2,f'c{p}')+s(3,path))
        for a in range(agents_per):
            aid=f'agent-{p*agents_per+a+1}'
            put(30, s(1,aid)+s(2,pid)+s(3,node)+s(4,f'c{p}')+s(5,['claude-code','codex','wrongstack'][a%3]))
            put(31, s(1,aid)+v(2,[1,2,3,4][a%4])+v(3,1))
    db.execute('INSERT INTO producers(producer,last_seq) VALUES(?,?) ON CONFLICT(producer) DO UPDATE SET last_seq=excluded.last_seq',(node,seq))
db.commit()
print('events', db.execute('select count(*) from events').fetchone()[0])
