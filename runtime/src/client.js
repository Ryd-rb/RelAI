// Generic renderer for views returned by a checked FluxR application.
const root=document.querySelector('main');
const token=location.hash.slice(1)||sessionStorage.getItem('session')||'';
sessionStorage.setItem('session',token);
history.replaceState(null,'',location.pathname);
const state=JSON.parse(sessionStorage.getItem('view')||'{"path":""}');
let revision=0, last='', busy=false;
const drafts=new Map();
const element=(tag,text)=>{const e=document.createElement(tag);if(text!==undefined)e.textContent=text;return e};
async function call(op,arg){
 const response=await fetch('/api/'+encodeURIComponent(op),{method:'POST',cache:'no-store',headers:{Authorization:'Bearer '+token,'Content-Type':'application/json'},body:JSON.stringify(arg)});
 if(!response.ok)throw Error(await response.text());return response.json();
}
function notice(message){let status=document.querySelector('#status');if(!status){status=element('output');status.id='status';status.setAttribute('role','status');root.prepend(status)}status.textContent=message}
function navigate(values){
 if(values.project && values.project!==state.project){state.path='';state.reply=null;state.steer=null;state.snapshot=null;state.selected=null}
 Object.assign(state,values);sessionStorage.setItem('view',JSON.stringify(state));last='';refresh();
}
async function execute(control,data){
 const result=await call(control.op,{...data,...control.fixed});
 if(control.selectResult){const values={};for(const [key,source] of Object.entries(control.selectResult))values[key]=result[source];navigate({view:'relay',...control.after,...values});}
 return result;
}
function show(value){return typeof value==='string'?value:JSON.stringify(value,null,2)}
function termCompletion(input,label,project,config,draft){
 const host=element('div');host.className='term-completion';
 const description=element('div');description.setAttribute('aria-live','polite');
 const choices=element('div');choices.setAttribute('role','group');choices.setAttribute('aria-label','Matching terms');
 host.append(description,choices);label.append(host);
 let sequence=0,timer,prior=input.value;
 const length=s=>new TextEncoder().encode(s).length;
 const changed=()=>{
   const now=input.value;let begin=0,end=0;
   while(begin<prior.length && begin<now.length && prior[begin]===now[begin])begin++;
   while(end<prior.length-begin && end<now.length-begin && prior[prior.length-1-end]===now[now.length-1-end])end++;
   const oldStop=prior.length-end,delta=now.length-prior.length;
   draft.bindings=(draft.bindings||[]).flatMap(m=>m.end<=begin?[m]:m.start>=oldStop?[{...m,start:m.start+delta,end:m.end+delta}]:[]);
   prior=now;refreshChoices();
 };
 const refreshChoices=()=>{
   clearTimeout(timer);const id=++sequence;choices.replaceChildren();description.textContent='';
   const stop=input.selectionStart;const prefix=input.value.slice(0,stop).match(/[\p{L}\p{N}_-]+$/u)?.[0];
   if(!prefix||input.selectionEnd!==stop)return;
   const start=stop-prefix.length;
   timer=setTimeout(async()=>{try{
     const list=await call(config.action,{project,prefix});if(id!==sequence||input.selectionStart!==stop)return;
     description.textContent=list[0]?.description||'';
     for(const hit of list){const button=element('button',hit.name+' · '+hit.kind+' · '+hit.dictionary_name);
       button.type='button';button.dataset.kind=hit.kind;button.onfocus=button.onmouseenter=()=>{description.textContent=hit.description};
       button.onclick=()=>{input.setRangeText(hit.name,start,stop,'end');changed();draft.values[input.name]=input.value;
         const end=start+hit.name.length;draft.bindings=(draft.bindings||[]).filter(m=>m.end<=start||m.start>=end);
         draft.bindings.push({start,end,ref:{id:hit.id,revision:hit.revision,dictionary_id:hit.dictionary_id}});
         clearTimeout(timer);++sequence;choices.replaceChildren();description.textContent=hit.description;input.focus();};
       choices.append(button)}
   }catch(err){if(id===sequence)description.textContent=err.message}},120);
 };
 input.addEventListener('input',changed);
 input.addEventListener('keydown',e=>{if(e.key==='ArrowDown'&&choices.firstChild){e.preventDefault();choices.firstChild.focus()}if(e.key==='Escape'){clearTimeout(timer);++sequence;choices.replaceChildren();description.textContent=''}});
 return ()=>[...(draft.bindings||[])].sort((a,b)=>a.start-b.start).map(m=>({ref:m.ref,start:length(input.value.slice(0,m.start)),end:length(input.value.slice(0,m.end))}));
}
function termText(parts,open) {
 const paragraph=element('p');
 for(const part of parts) {
   if(!part.term){paragraph.append(document.createTextNode(part.text));continue}
   const link=element('button',part.text);link.type='button';link.className='term';link.dataset.kind=part.term.kind;
   link.setAttribute('aria-label',part.text+' · '+part.term.kind+' definition');link.onclick=()=>open(part.term);paragraph.append(link);
 }
 return paragraph;
}
function openDefinition(project,first) {
 const dialog=element('dialog');dialog.setAttribute('aria-label','Term definition');document.body.append(dialog);
 const stack=[first];let generation=0;dialog.onclose=()=>{++generation;dialog.remove()};dialog.showModal();
 const display=async()=>{const ticket=++generation;const ref=stack.at(-1);try{
   const item=await call('term_view',{project,ref});if(ticket!==generation||!dialog.open)return;
   dialog.replaceChildren();const close=element('button','Close');close.onclick=()=>dialog.close();dialog.append(close);
   if(stack.length>1){const back=element('button','Back');back.onclick=()=>{stack.pop();display()};dialog.append(back)}
   if(!item.available){dialog.append(element('p','This definition is unavailable.'));return}
   dialog.append(element('h2',item.entry.name),element('p',item.dictionary.name+' · '+item.entry.kind+' · revision '+item.entry.revision));
   dialog.append(termText(item.parts,target=>{if(stack.length>=32)stack.shift();stack.push(target);display()}));
   if(item.dictionary.owner){
     const edit=element('details');edit.append(element('summary','Edit owner definition'));
     const owner=item.enabled_projects.find(p=>p.id===item.dictionary.owner);
     edit.append(element('p','Dictionary owner · '+(owner?.root||item.dictionary.owner)),element('p','Enabled projects · '+item.enabled_projects.map(p=>p.root).join(', ')));
     const label=element('label','Definition');const field=element('textarea');field.value=item.entry.definition;label.append(field);edit.append(label);
     const save=element('button','Save revision');edit.append(save);save.onclick=async()=>{save.disabled=true;try{
       const r=await call('dictionary',{project:item.dictionary.owner,op:'save',dictionary:item.dictionary.id,id:item.entry.id,revision:item.entry.revision,name:item.entry.name,kind:item.entry.kind,definition:field.value});
       if(ticket===generation){stack[stack.length-1]=r;await display()}
     }catch(err){edit.append(element('p',err.message))}finally{save.disabled=false}};dialog.append(edit);
   }
 }catch(err){if(ticket===generation){dialog.close();notice(err.message)}}};display();
}
function render(spec,project){
 if(spec.type==='fold'){const fold=element('details');fold.append(element('summary',spec.label));for(const item of spec.items)fold.append(render(item,project));return fold}
 if(spec.type==='graph_layers'){
 const section=element('section');section.append(element('h2',spec.label));const select=element('select');select.setAttribute('aria-label','Compare with snapshot');
 const none=element('option','Current snapshot');none.value='';select.append(none);for(const version of spec.versions||[]){if(version.id===spec.snapshot)continue;const choice=element('option',version.id);choice.value=version.id;select.append(choice)}
 const load=element('button','Inspect source relationships');const detail=element('div');section.append(select,load,detail);
 const item=(title,value)=>{const block=element('details');block.append(element('summary',title),element('pre',show(value)));detail.append(block)};
 let ticket=0;const query=async()=>{const id=++ticket;try{const v=await call(spec.op,{project,snapshot:spec.snapshot,before:select.value||null});if(id!==ticket||!section.isConnected)return;
 detail.replaceChildren(element('p',v.coverage.indexed+' of '+v.coverage.total+' source files indexed.'));for(const edge of v.edges){const b=element('button',edge.source+' → '+edge.target+' · '+edge.name);b.onclick=()=>navigate({view:'graph',snapshot:spec.snapshot,selected:edge.source});detail.append(b)}
 if(v.changes){item('Changes since selected snapshot',v.changes);item('Stale source evidence',v.stale_evidence);for(const change of v.changes.filter(c=>c.old)){const b=element('button','Open previous '+change.path);b.onclick=()=>navigate({view:'graph',snapshot:select.value,selected:change.path});detail.append(b)}}
 item('Recorded activity',v.activity)}catch(err){if(id===ticket)notice(err.message)}};load.onclick=select.onchange=query;return section;
 }
 if(spec.type==='tree'){
 const container=element('section');container.append(element('h2',spec.label));
 const nodes=spec.nodes||[];const byParent=new Map();
 for(const item of nodes){if(!byParent.has(item.parent))byParent.set(item.parent,[]);byParent.get(item.parent).push(item)}
 const search=element('input');search.type='search';search.setAttribute('aria-label','Filter paths');container.append(search);
 const body=element('div');body.className='tree-body';container.append(body);
 const pause=element('button',state.graphPaused?'Resume graph':'Pause graph');pause.onclick=()=>{state.graphPaused=!state.graphPaused;pause.textContent=state.graphPaused?'Resume graph':'Pause graph';if(!state.graphPaused)refresh(true)};container.insertBefore(pause,body);
 let drag;body.addEventListener('pointerdown',e=>{if(e.target.closest('button'))return;drag=[e.clientX,e.clientY,body.scrollLeft,body.scrollTop];body.setPointerCapture(e.pointerId)});body.addEventListener('pointermove',e=>{if(drag){body.scrollLeft=drag[2]+drag[0]-e.clientX;body.scrollTop=drag[3]+drag[1]-e.clientY}});body.addEventListener('pointerup',()=>drag=null);body.addEventListener('pointercancel',()=>drag=null);
 let zoom=1;for(const [label,factor] of [['Zoom in',1.2],['Zoom out',1/1.2],['Fit',0]]){const b=element('button',label);b.onclick=()=>{zoom=factor?Math.max(.5,Math.min(3,zoom*factor)):1;body.style.fontSize=zoom+'em'};container.insertBefore(b,body)}
 const draw=()=>{body.replaceChildren();const ul=element('ul');ul.setAttribute('role','tree');body.append(ul);
 const add=(parent,target)=>{for(const item of byParent.get(parent)||[]){if(search.value && !item.path.includes(search.value) && !nodes.some(n=>n.path.startsWith(item.path+'/')&&n.path.includes(search.value)))continue;
 const li=element('li');li.setAttribute('role','treeitem');const b=element('button',item.path.split('/').pop());b.title=item.path;b.onclick=()=>navigate({...spec.navigate,[spec.pathField]:item.path});li.append(b);
 if(item.kind==='directory'){const group=element('ul');group.setAttribute('role','group');li.append(group);add(item.path,group)}target.append(li)}};
 add('',ul)};search.oninput=draw;draw();return container;
 }
 if(spec.type==='button'){
 const b=element('button',spec.label);b.type='button';
 b.onclick=async()=>{try{if(spec.navigate){navigate(spec.navigate);return}b.disabled=true;await execute(spec,{project});await refresh(true)}catch(e){notice(e.message)}finally{b.disabled=false}};return b;
 }
 if(spec.type==='form'){
 const form=element('form');const fields=new Map();
 const draftId=JSON.stringify([project,spec.op,spec.fixed]);let draft=drafts.get(draftId)||{values:{},files:[],key:crypto.randomUUID()};
 if(spec.draft){drafts.set(draftId,draft);form.className='composer'}
 let markerList;let pending=0;const submit=element('button',spec.label);submit.type='submit';
 for(const field of spec.fields){
 const label=element('label');label.append(element('span',field.label));let input;
 if(field.type==='select'||field.type==='multi'){input=element('select');input.multiple=field.type==='multi';for(const option of field.choices){const o=element('option',option.label??option);o.value=option.value??option;input.append(o)}}
 else if(field.type==='textarea'||field.type==='json'){input=element('textarea');if(field.type==='json')input.placeholder='{}'}
 else{input=element('input');input.type=field.type==='files'?'file':field.type==='boolean'?'checkbox':'text'}
 input.required=!!field.required;input.name=field.name;
 if(field.type==='files'){
 input.multiple=true;const list=element('ul');const update=()=>{list.replaceChildren();for(const file of draft.files){const li=element('li',file.name+' · '+file.status+' ');const remove=element('button','Remove');remove.type='button';remove.onclick=()=>{draft.files=draft.files.filter(x=>x!==file);update()};li.append(remove);list.append(li)}submit.disabled=pending>0||draft.files.some(x=>!x.id)};
 const capture=async(files)=>{for(const f of files){const item={name:f.name,status:'Loading'};draft.files.push(item);pending++;update();try{
 if(f.size>8*1024*1024)throw Error('Exceeds 8 MiB');
 const content=await new Promise((resolve,reject)=>{const reader=new FileReader();reader.onerror=()=>reject(Error('Unable to read file'));reader.onload=()=>resolve(reader.result.split(',')[1]);reader.readAsDataURL(f)});
 const result=await call(field.op,{project,name:f.name,mime:f.type||'text/plain',base64:content});item.id=result.id;item.status=f.size+' bytes';
 }catch(e){item.status=e.message}finally{pending--;update()}}};
 input.onchange=()=>capture(Array.from(input.files));form.ondragover=e=>e.preventDefault();form.ondrop=e=>{e.preventDefault();capture(Array.from(e.dataTransfer.files))};label.append(input,list);update();
 }else{if(field.name in draft.values)input.value=draft.values[field.name];input.oninput=()=>{draft.values[field.name]=input.value};label.append(input)}
 if(field.type==='folder'){const pick=element('button','Browse');pick.type='button';pick.onclick=async()=>{try{const choice=await call('choose_folder',{});if(choice.path){input.value=choice.path;draft.values[field.name]=choice.path}}catch(e){notice(e.message)}};label.append(pick)}
 if(spec.completion?.field===field.name)markerList=termCompletion(input,label,project,spec.completion,draft);
 fields.set(field.name,[field,input]);form.append(label);
 }
 form.append(submit);const output=element('pre');form.append(output);
 form.onsubmit=async e=>{e.preventDefault();if(busy||pending)return;busy=true;submit.disabled=true;try{
 const data={project};if(spec.idempotent)data.key=draft.key;
 for(const [name,[field,input]] of fields)data[name]=field.type==='boolean'?input.checked:field.type==='files'?draft.files.map(x=>x.id):field.type==='json'?JSON.parse(input.value||'{}'):field.type==='multi'?Array.from(input.selectedOptions,o=>o.value):input.value;
 if(markerList)data[spec.completion.markers]=markerList();
 const payload=spec.jsonPayload?{...data[spec.jsonPayload],project}:data;
 const result=await execute(spec,payload);
 if(spec.draft){drafts.delete(draftId);state.reply=null;state.steer=null;form.reset()}
 if(spec.showResult){output.textContent=show(result)}else await refresh(true);
 }catch(e){notice(e.message)}finally{busy=false;submit.disabled=false}};return form;
 }
 const node=element(spec.type==='nav'?'nav':spec.type==='card'?'article':'section');node.className=spec.type;
 if(spec.type==='nav')node.setAttribute('aria-label',spec.label);
 if(spec.dockable){const applyDock=()=>{root.dataset.dock=sessionStorage.getItem('dock')||'top'};applyDock();node.dataset.dockable='true';
 const dock=element('button',root.dataset.dock==='left'?'Move tabs to top':'Move tabs to left');
 dock.onclick=()=>{sessionStorage.setItem('dock',root.dataset.dock==='left'?'top':'left');applyDock();dock.textContent=root.dataset.dock==='left'?'Move tabs to top':'Move tabs to left'};node.append(dock);}
 else if(spec.label)node.append(element(spec.type==='card'?'h3':'h2',spec.label));
 if(spec.parts?.length)node.append(termText(spec.parts,term=>openDefinition(project,term)));else if(spec.text)node.append(element('p',spec.text));
 if(spec.details){const d=element('details');d.append(element('summary','Details'),element('pre',show(spec.details)));node.append(d)}
 if('value' in spec)node.append(element('pre',show(spec.value)));
 for(const child of spec.items||[])node.append(render(child,project));return node;
}
async function refresh(force=false){
 const run=++revision;try{
 const view=await call('dashboard',state);if(run!==revision)return;
 const snapshot=JSON.stringify(view);
 if(!force && (snapshot===last||['INPUT','TEXTAREA','SELECT'].includes(document.activeElement?.tagName)||busy))return;
 if(view.project)state.project=view.project;last=snapshot;document.title=view.title;
 const head=element('header');head.append(element('h1',view.title),element('span',view.subtitle));
 root.replaceChildren(head,...view.components.map(c=>render(c,view.project)));
 }catch(e){notice(e.message)}
}
refresh();setInterval(()=>{if(!document.hidden&&!busy&&!(state.view==='graph'&&state.graphPaused))refresh()},2500);
