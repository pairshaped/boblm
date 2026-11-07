"""Pi 0.73.1 RPC shapes recorded in docs/live-pi-protocol.md."""
import json, os, sys, uuid
mode=os.environ.get('FAKE_MODE','normal')
assert os.environ['DEEPINFRA_API_KEY']=='fictional-provider-key'
assert all('fictional-provider-key' not in arg for arg in sys.argv)
with open(os.path.join(os.environ['PI_CODING_AGENT_DIR'],'settings.json')) as f:settings=json.load(f)
assert settings['retry']=={'enabled':False,'provider':{'maxRetries':2}}
assert settings['compaction']['enabled'] is False
state={'sessionFile':'/fake/'+str(uuid.uuid4())+'.jsonl','model':{'provider':'deepinfra','id':'deepseek-ai/DeepSeek-V4.1-Flash-1017'},'thinkingLevel':'low'}
if '--session' in sys.argv and mode!='wrong_resume':state['sessionFile']=sys.argv[sys.argv.index('--session')+1]
if mode=='exit_start':sys.exit(2)
if mode=='fork_child':
 import subprocess
 child=subprocess.Popen(['sleep','120'])
 with open(os.environ['FAKE_PIDS'],'w') as f:f.write(str(os.getpid())+'\n'+str(child.pid))
active=False
count=0
held=[]
def out(v):print(json.dumps(v),flush=True)
def reply(r,data=None,error=None):
 v=dict(type='response',id=r['id'],command=r['type'],success=error is None)
 if data is not None:v['data']=data
 if error:v['error']=error
 out(v)
def user(text):
 m={'role':'user','content':[{'type':'text','text':text}]}
 out({'type':'message_start','message':m});out({'type':'message_end','message':m})
def finish(text,reason='stop',tool=False):
 global active
 content=[{'type':'text','text':text}] if not tool else [{'type':'toolCall','id':'call-1','name':'bash','arguments':{'command':'false'}}]
 a={'role':'assistant','content':content,'usage':{'input':42,'output':3,'cacheRead':10,'cacheWrite':0,'totalTokens':55},'stopReason':reason}
 if reason=='error':
  a['errorMessage']="This model's maximum context length is 1048576 tokens. However, you requested more." if mode=='overflow' else 'provider error fictional-provider-key'
  if mode=='json_error':a['errorMessage']='429 '+json.dumps({'error':{'message':'rate limit fictional-provider-key'}})+'\n at provider.js:10'
 out({'type':'message_start','message':dict(a,content=[])})
 out({'type':'message_update','assistantMessageEvent':{'type':'text_delta','delta':text,'contentIndex':0}})
 out({'type':'message_end','message':a})
 if tool:
  out({'type':'tool_execution_end','toolCallId':'call-1','toolName':'bash','result':{'content':[{'type':'text','text':'exit 1'}]},'isError':True})
  out({'type':'turn_end','message':a,'toolResults':[]});finish('tool finished');return
 out({'type':'turn_end','message':a,'toolResults':[]});out({'type':'agent_end','messages':[a]});active=False
for line in sys.stdin:
 r=json.loads(line);t=r['type']
 if t=='get_state':reply(r,state)
 elif t=='get_session_stats':reply(r,{'contextUsage':{'tokens':84000,'contextWindow':200000,'percent':42},'tokens':{'total':9999999}})
 elif t=='set_model':state['model']={'provider':r['provider'],'id':r['modelId']};reply(r,state['model'])
 elif t=='set_thinking_level':state['thinkingLevel']='high' if mode=='bad_effort' else r['level'];reply(r)
 elif t=='prompt':
  assert r['streamingBehavior']=='steer'
  if r['message']=='refuse':reply(r,error='no model');continue
  if mode=='duplicate_refusal' and count==1:
   reply(r,error='fixture refuses the second duplicate');count+=1;continue
  reply(r);count+=1
  if mode in ('duplicates','duplicate_refusal'):
   held.append(r['message'])
   if count==1:continue
   out({'type':'agent_start'})
   for text in held:user(text)
   finish('duplicates consumed');held.clear();continue
  if mode=='ack_only':sys.exit(0)
  if mode=='unmatched':
   sys.stdout.buffer.write(b'not json\n\xff\n'+b'x'*(16*1024*1024+1)+b'\n');sys.stdout.flush()
   out({'type':'future_event'});out({'type':'turn_start'});user('different');finish('unrelated');continue
  if not active:out({'type':'agent_start'});out({'type':'turn_start'})
  if mode=='hold_forever' and active:continue
  user(r['message']);active=True
  if mode in ('hold_forever','abort_ack_only') or (mode=='hold' and count==1):continue
  if r['message']=='ask':out({'type':'extension_ui_request','id':'q-1','method':'confirm'});continue
  finish('echo: '+r['message'],'error' if r['message']=='fail' else 'stop',r['message']=='tool')
 elif t=='extension_ui_response':
  assert r['cancelled'] is True;finish('canceled')
 elif t=='abort':
  if mode=='abort_ack_only':reply(r);sys.exit(0)
  finish('stopped','aborted');reply(r)
 elif t=='compact':
  if mode=='compact_ack_only':reply(r);sys.exit(0)
  out({'type':'compaction_start','reason':'manual'})
  out({'type':'compaction_end','reason':'manual','result':{'summary':'summary'},'aborted':False,'willRetry':False});reply(r,{'summary':'summary'})
 elif t=='new_session':
  if mode!='clear_same_file':state['sessionFile']='/fake/'+str(uuid.uuid4())+'.jsonl'
  reply(r,{'cancelled':False})
 elif t=='get_commands':reply(r,{'commands':[] if mode=='missing_tools' else [{'name':'boblm-peer-tools-ready'}]})
 else:reply(r)
