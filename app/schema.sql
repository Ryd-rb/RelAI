CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY,root TEXT NOT NULL UNIQUE,discriminator TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS desktop_selection(singleton INTEGER PRIMARY KEY CHECK(singleton=1),project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS events(cursor INTEGER PRIMARY KEY AUTOINCREMENT,project TEXT NOT NULL REFERENCES projects,kind TEXT NOT NULL,payload TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS project_cursor ON events(project,cursor);
CREATE TABLE IF NOT EXISTS event_times(cursor INTEGER PRIMARY KEY REFERENCES events(cursor) ON DELETE CASCADE,occurred INTEGER NOT NULL);
CREATE TRIGGER IF NOT EXISTS timestamp_new_event AFTER INSERT ON events BEGIN
 INSERT INTO event_times(cursor,occurred) VALUES(NEW.cursor,unixepoch());
END;
CREATE TABLE IF NOT EXISTS presets(scope TEXT NOT NULL,role TEXT NOT NULL,provider TEXT NOT NULL,model TEXT NOT NULL,effort TEXT NOT NULL,rev INTEGER NOT NULL,PRIMARY KEY(scope,role));
CREATE TABLE IF NOT EXISTS artifacts(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,meta TEXT NOT NULL,encoded TEXT NOT NULL,UNIQUE(project,id));
CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,revision INTEGER NOT NULL DEFAULT 1,state TEXT NOT NULL DEFAULT 'open',intent TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS submissions(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,request TEXT NOT NULL REFERENCES requests,key TEXT NOT NULL,hash TEXT NOT NULL,content TEXT NOT NULL,UNIQUE(project,key));
CREATE INDEX IF NOT EXISTS submissions_request ON submissions(request);
CREATE INDEX IF NOT EXISTS chat_cursor ON events(project,cursor) WHERE kind IN ('Submission','ASK','RESULT','Final result','Request stopped');
CREATE INDEX IF NOT EXISTS chat_request_cursor ON events(project,(payload->>'$.request'),cursor)
  WHERE kind IN ('Submission','ASK','RESULT','Final result','Request stopped');
CREATE TABLE IF NOT EXISTS files(path TEXT PRIMARY KEY,content TEXT NOT NULL,digest TEXT NOT NULL,verified INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,request TEXT NOT NULL REFERENCES requests,revision INTEGER NOT NULL,session TEXT NOT NULL,role TEXT NOT NULL,kind TEXT NOT NULL,input TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'queued',seal TEXT,attempt TEXT,receipt TEXT,reason TEXT);
CREATE INDEX IF NOT EXISTS queue ON jobs(state,role,session);
CREATE TABLE IF NOT EXISTS attempts(id TEXT PRIMARY KEY,job TEXT NOT NULL REFERENCES jobs,started INTEGER NOT NULL DEFAULT(unixepoch()),ended INTEGER,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS actions(id TEXT PRIMARY KEY,job TEXT NOT NULL REFERENCES jobs,kind TEXT NOT NULL,payload TEXT NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS branches(id TEXT PRIMARY KEY,request TEXT NOT NULL REFERENCES requests,revision INTEGER NOT NULL,fan TEXT NOT NULL,parent TEXT,role TEXT NOT NULL,task TEXT NOT NULL,report TEXT,accepted TEXT);
CREATE TABLE IF NOT EXISTS reviews(id TEXT PRIMARY KEY,request TEXT NOT NULL REFERENCES requests,revision INTEGER NOT NULL,fan TEXT NOT NULL,signature TEXT NOT NULL,job TEXT NOT NULL,UNIQUE(fan,revision,signature));
CREATE TABLE IF NOT EXISTS questions(id TEXT PRIMARY KEY,request TEXT NOT NULL REFERENCES requests,revision INTEGER NOT NULL,session TEXT NOT NULL,role TEXT NOT NULL,body TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'open');
CREATE TABLE IF NOT EXISTS finals(request TEXT PRIMARY KEY REFERENCES requests,action TEXT NOT NULL,path TEXT NOT NULL,released INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS improvements(request TEXT PRIMARY KEY REFERENCES requests,session TEXT UNIQUE NOT NULL,job TEXT UNIQUE NOT NULL);
CREATE TABLE IF NOT EXISTS terms(id TEXT PRIMARY KEY,project TEXT NOT NULL,name TEXT NOT NULL,definition TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 1,kind TEXT NOT NULL DEFAULT 'knowledge');
CREATE TABLE IF NOT EXISTS snapshots(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,manifest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS passages(project TEXT NOT NULL,digest TEXT NOT NULL,start INTEGER NOT NULL,end INTEGER NOT NULL,body TEXT NOT NULL,PRIMARY KEY(project,digest,start));
CREATE TABLE IF NOT EXISTS passage_sources(project TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(project,digest));
CREATE TABLE IF NOT EXISTS blobs(project TEXT NOT NULL,digest TEXT NOT NULL,content TEXT NOT NULL,PRIMARY KEY(project,digest));

CREATE TABLE IF NOT EXISTS tools(version TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,name TEXT NOT NULL,source TEXT NOT NULL,export TEXT NOT NULL,contract TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'draft',runtime TEXT NOT NULL,evidence TEXT);
CREATE TABLE IF NOT EXISTS tool_tests(id TEXT PRIMARY KEY,project TEXT NOT NULL,version TEXT NOT NULL REFERENCES tools,request TEXT NOT NULL,expected TEXT NOT NULL,observed TEXT,outcome TEXT NOT NULL,receipt TEXT NOT NULL,provisional INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS tool_defaults(project TEXT NOT NULL,name TEXT NOT NULL,version TEXT NOT NULL REFERENCES tools,PRIMARY KEY(project,name));
CREATE TABLE IF NOT EXISTS inventory(snapshot TEXT NOT NULL,path TEXT NOT NULL,parent TEXT NOT NULL,kind TEXT NOT NULL,digest TEXT,bytes INTEGER NOT NULL,PRIMARY KEY(snapshot,path));
CREATE TABLE IF NOT EXISTS dictionaries(id TEXT PRIMARY KEY,owner TEXT,name TEXT NOT NULL,archived INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS dictionary_links(project TEXT NOT NULL REFERENCES projects,dictionary TEXT NOT NULL REFERENCES dictionaries,enabled INTEGER NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(project,dictionary));
CREATE TABLE IF NOT EXISTS entries(id TEXT NOT NULL,revision INTEGER NOT NULL,dictionary TEXT NOT NULL REFERENCES dictionaries,name TEXT NOT NULL,kind TEXT NOT NULL,definition TEXT NOT NULL,source TEXT NOT NULL,human INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(id,revision));
CREATE TABLE IF NOT EXISTS skills(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,name TEXT NOT NULL,digest TEXT NOT NULL,description TEXT NOT NULL,body TEXT NOT NULL,resources TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS selected_sources(request TEXT NOT NULL REFERENCES requests,revision INTEGER NOT NULL,session TEXT NOT NULL,path TEXT NOT NULL,PRIMARY KEY(request,revision,session,path));
CREATE TABLE IF NOT EXISTS registry_terms(project TEXT NOT NULL,name TEXT NOT NULL,entry TEXT NOT NULL UNIQUE,contract TEXT NOT NULL,PRIMARY KEY(project,name));
CREATE TABLE IF NOT EXISTS term_tool_versions(entry TEXT NOT NULL,revision INTEGER NOT NULL,tool_version TEXT,PRIMARY KEY(entry,revision));
CREATE TABLE IF NOT EXISTS definition_parts(entry TEXT NOT NULL,revision INTEGER NOT NULL,parts TEXT NOT NULL,PRIMARY KEY(entry,revision));
INSERT OR IGNORE INTO registry_terms SELECT d.project,d.name,lower(hex(randomblob(16))),t.contract FROM tool_defaults d JOIN tools t ON t.version=d.version;
INSERT INTO entries SELECT r.entry,1,r.project,r.name,'tool','Checked implementation of '||r.contract,'registry',0 FROM registry_terms r WHERE NOT EXISTS(SELECT 1 FROM entries e WHERE e.id=r.entry);
INSERT OR IGNORE INTO term_tool_versions SELECT e.id,e.revision,d.version FROM registry_terms r JOIN entries e ON e.id=r.entry LEFT JOIN tool_defaults d ON d.project=r.project AND d.name=r.name WHERE e.revision=(SELECT max(revision) FROM entries WHERE id=r.entry);
CREATE TABLE IF NOT EXISTS skill_origins(version TEXT NOT NULL REFERENCES skills,identity TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(version,identity));
CREATE TABLE IF NOT EXISTS skill_dependencies(version TEXT NOT NULL REFERENCES skills,name TEXT NOT NULL,tool_version TEXT NOT NULL,required INTEGER NOT NULL,PRIMARY KEY(version,name));
CREATE TABLE IF NOT EXISTS skill_drafts(version TEXT NOT NULL REFERENCES skills,path TEXT NOT NULL,source TEXT NOT NULL,contract TEXT NOT NULL,tests TEXT NOT NULL,PRIMARY KEY(version,path));
CREATE TABLE IF NOT EXISTS proposals(id TEXT PRIMARY KEY,project TEXT NOT NULL,entry TEXT,base_revision INTEGER,proposed TEXT NOT NULL,source TEXT NOT NULL,status TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS proposal_terms(proposal TEXT PRIMARY KEY REFERENCES proposals,dictionary TEXT NOT NULL REFERENCES dictionaries,name TEXT NOT NULL,kind TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS candidate_versions(candidate TEXT PRIMARY KEY REFERENCES candidates,version TEXT NOT NULL,baseline TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS verified_checks(test TEXT PRIMARY KEY REFERENCES tool_tests,version TEXT NOT NULL,project TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS publications(id TEXT PRIMARY KEY,project TEXT NOT NULL,request TEXT NOT NULL,candidate TEXT NOT NULL UNIQUE REFERENCES candidates,receipt TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS plans(project TEXT NOT NULL,request TEXT NOT NULL,revision INTEGER NOT NULL,body TEXT NOT NULL,PRIMARY KEY(project,request));
CREATE TABLE IF NOT EXISTS candidates(id TEXT PRIMARY KEY,project TEXT NOT NULL,operation TEXT NOT NULL,description TEXT NOT NULL,evidence TEXT NOT NULL,checks TEXT NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS schedules(id TEXT PRIMARY KEY,project TEXT NOT NULL,revision INTEGER NOT NULL,due INTEGER NOT NULL,period INTEGER NOT NULL,action TEXT NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS monitors(id TEXT PRIMARY KEY REFERENCES schedules,project TEXT NOT NULL REFERENCES projects,latest TEXT,updated INTEGER);
CREATE TABLE IF NOT EXISTS occurrences(schedule TEXT NOT NULL,revision INTEGER NOT NULL,instant INTEGER NOT NULL,state TEXT NOT NULL,PRIMARY KEY(schedule,revision,instant));
CREATE TABLE IF NOT EXISTS prerequisites(job TEXT NOT NULL REFERENCES jobs, reference TEXT NOT NULL, kind TEXT NOT NULL, PRIMARY KEY(job,reference));
CREATE VIEW IF NOT EXISTS current_prerequisites AS
 SELECT j.id AS job,p.reference,p.kind,
 CASE p.kind WHEN 'answer' THEN EXISTS(SELECT 1 FROM questions q WHERE q.id=p.reference AND q.request=j.request AND q.revision=j.revision AND q.state='answered')
 WHEN 'acceptance' THEN EXISTS(SELECT 1 FROM branches b WHERE b.id=p.reference AND b.request=j.request AND b.revision=j.revision AND b.accepted IS NOT NULL)
 WHEN 'artifact' THEN EXISTS(SELECT 1 FROM artifacts a WHERE a.id=p.reference AND a.project=j.project)
 ELSE 0 END AS satisfied FROM prerequisites p JOIN jobs j ON j.id=p.job;
CREATE VIEW IF NOT EXISTS branch_outcomes AS
 SELECT b.*,a.payload AS result,
 (SELECT json_group_array(id) FROM (SELECT id FROM questions q WHERE q.session=b.id AND q.request=b.request AND q.revision=b.revision AND q.state!='answered' ORDER BY id)) AS questions,
 (SELECT json_group_array(reference) FROM (SELECT c.reference FROM current_prerequisites c JOIN jobs w ON c.job=w.id WHERE w.session=b.id AND w.request=b.request AND w.revision=b.revision AND w.state='waiting' AND c.satisfied=0 ORDER BY c.reference)) AS waits,
 (SELECT json_group_array(json_object('job',id,'state',state,'reason',reason)) FROM (SELECT id,state,reason FROM jobs j WHERE j.session=b.id AND j.request=b.request AND j.revision=b.revision AND j.state IN ('held','uncertain') ORDER BY id)) AS holds,
 EXISTS(SELECT 1 FROM jobs j WHERE j.session=b.id AND j.request=b.request AND j.revision=b.revision AND j.state IN ('running','queued','sealing','sealed','executing')) AS busy
 FROM branches b LEFT JOIN actions a ON a.id=b.report;
CREATE VIEW IF NOT EXISTS branch_descendants AS
 WITH RECURSIVE tree(ancestor,child) AS (SELECT parent,id FROM branches WHERE parent IS NOT NULL UNION SELECT t.ancestor,b.id FROM tree t JOIN branches b ON b.parent=t.child) SELECT ancestor,child FROM tree;
CREATE TABLE IF NOT EXISTS managed_worktrees(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects(id),operation TEXT NOT NULL,request TEXT NOT NULL,digest TEXT NOT NULL,state TEXT NOT NULL,grant_id TEXT,result TEXT,error TEXT);
CREATE TABLE IF NOT EXISTS worktree_grants(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects(id),worktree TEXT NOT NULL REFERENCES managed_worktrees(id),digest TEXT NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS git_mutations(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,operation TEXT NOT NULL,request TEXT NOT NULL,digest TEXT NOT NULL,state TEXT NOT NULL,grant_id TEXT,result TEXT,error TEXT);
CREATE TABLE IF NOT EXISTS git_grants(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,mutation TEXT NOT NULL REFERENCES git_mutations,digest TEXT NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS graph_symbols(snapshot TEXT NOT NULL,path TEXT NOT NULL,digest TEXT NOT NULL,language TEXT NOT NULL,role TEXT NOT NULL,kind TEXT,name TEXT,start INTEGER,end INTEGER);
CREATE INDEX IF NOT EXISTS graph_symbol_lookup ON graph_symbols(snapshot,language,role,name);
CREATE TABLE IF NOT EXISTS graph_indexes(snapshot TEXT PRIMARY KEY,indexed INTEGER NOT NULL,total INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS graph_index_versions(snapshot TEXT PRIMARY KEY REFERENCES graph_indexes(snapshot) ON DELETE CASCADE,version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS graph_cursors(snapshot TEXT PRIMARY KEY,cursor INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS file_effects(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,operation TEXT NOT NULL,request TEXT NOT NULL,digest TEXT NOT NULL,state TEXT NOT NULL,grant_id TEXT,result TEXT,error TEXT);
CREATE TABLE IF NOT EXISTS file_grants(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,effect TEXT NOT NULL REFERENCES file_effects,digest TEXT NOT NULL,destructive INTEGER NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS patch_stages(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,request TEXT NOT NULL,digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS chat_patch_reviews(effect TEXT PRIMARY KEY REFERENCES file_effects(id),job TEXT NOT NULL UNIQUE REFERENCES jobs(id),request TEXT NOT NULL REFERENCES requests(id),revision INTEGER NOT NULL,files TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS chat_patch_events ON events(project,cursor) WHERE kind='Patch proposed';
CREATE TABLE IF NOT EXISTS git_remotes(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,url TEXT NOT NULL,branch TEXT NOT NULL,credential TEXT,active INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS git_transfers(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,remote TEXT NOT NULL REFERENCES git_remotes,operation TEXT NOT NULL,expected TEXT,target TEXT,state TEXT NOT NULL,receipt TEXT);
CREATE TABLE IF NOT EXISTS connectors(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,name TEXT NOT NULL,definition TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS network_effects(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,connector TEXT NOT NULL REFERENCES connectors,arguments TEXT NOT NULL,state TEXT NOT NULL,receipt TEXT);
CREATE VIEW IF NOT EXISTS open_chat_questions AS
SELECT r.project, q.id, q.request, q.rowid AS sequence, q.body->>'$.question' AS prompt
  FROM questions q JOIN requests r ON r.id=q.request AND r.revision=q.revision
 WHERE q.state='open' AND r.state='open' AND NOT EXISTS (
    SELECT 1 FROM jobs j WHERE j.request=r.id AND j.revision=r.revision AND j.kind='intake'
      AND j.input->>'$.reply'=q.id AND j.state IN ('queued','sealing','sealed','running','waiting','held','uncertain'));

CREATE VIEW IF NOT EXISTS chat_topics AS
SELECT s.request,s.project,coalesce(nullif(substr(topic,1,64)||CASE WHEN length(topic)>64 THEN '…' ELSE '' END,''),'Untitled request') AS topic
FROM (SELECT request,project,rowid,single_line(coalesce(nullif(content->>'$.text',''),content->>'$.attachments[0].display_name','')) AS topic
      FROM submissions) s
WHERE s.rowid=(SELECT min(f.rowid) FROM submissions f WHERE f.request=s.request);

CREATE VIEW IF NOT EXISTS pending_chat_answers AS
SELECT r.project, r.id AS request, r.intent AS user_message,
       a.id AS answer_id, a.payload->>'$.findings' AS answer
  FROM requests r JOIN jobs j ON j.request=r.id AND j.project=r.project AND j.revision=r.revision
  JOIN actions a ON a.job=j.id
 WHERE r.state='open' AND a.kind='RESULT' AND a.state='awaiting_acceptance'
   AND a.payload->>'$.result_kind'='final';
