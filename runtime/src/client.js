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
function render(spec,project){
 if(spec.type==='tree'){
 const container=element('section');container.append(element('h2',spec.label));
 const nodes=spec.nodes||[];const byParent=new Map();
 for(const item of nodes){if(!byParent.has(item.parent))byParent.set(item.parent,[]);byParent.get(item.parent).push(item)}
 const search=element('input');search.type='search';search.setAttribute('aria-label','Filter paths');container.append(search);
 const body=element('div');body.className='tree-body';container.append(body);
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
 let pending=0;const submit=element('button',spec.label);submit.type='submit';
 for(const field of spec.fields){
 const label=element('label');label.append(element('span',field.label));let input;
 if(field.type==='select'||field.type==='multi'){input=element('select');input.multiple=field.type==='multi';for(const option of field.choices){const o=element('option',option.label??option);o.value=option.value??option;input.append(o)}}
 else if(field.type==='textarea'||field.type==='json'){input=element('textarea');if(field.type==='json')input.placeholder='{}'}
 else{input=element('input');input.type=field.type==='files'?'file':'text'}
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
 fields.set(field.name,[field,input]);form.append(label);
 }
 form.append(submit);const output=element('pre');form.append(output);
 form.onsubmit=async e=>{e.preventDefault();if(busy||pending)return;busy=true;submit.disabled=true;try{
 const data={project};if(spec.idempotent)data.key=draft.key;
 for(const [name,[field,input]] of fields)data[name]=field.type==='files'?draft.files.map(x=>x.id):field.type==='json'?JSON.parse(input.value||'{}'):field.type==='multi'?Array.from(input.selectedOptions,o=>o.value):input.value;
 const result=await execute(spec,data);
 if(spec.draft){drafts.delete(draftId);state.reply=null;state.steer=null;form.reset()}
 if(spec.showResult){output.textContent=show(result)}else await refresh(true);
 }catch(e){notice(e.message)}finally{busy=false;submit.disabled=false}};return form;
 }
 const node=element(spec.type==='nav'?'nav':spec.type==='card'?'article':'section');node.className=spec.type;
 if(spec.type==='nav')node.setAttribute('aria-label',spec.label);
 else if(spec.label)node.append(element(spec.type==='card'?'h3':'h2',spec.label));
 if(spec.text)node.append(element('p',spec.text));
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
refresh();setInterval(()=>{if(!document.hidden&&!busy)refresh()},2500);
