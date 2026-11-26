CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY,root TEXT NOT NULL UNIQUE,discriminator TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS events(cursor INTEGER PRIMARY KEY AUTOINCREMENT,project TEXT NOT NULL REFERENCES projects,kind TEXT NOT NULL,payload TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS project_cursor ON events(project,cursor);
CREATE TABLE IF NOT EXISTS presets(scope TEXT NOT NULL,role TEXT NOT NULL,provider TEXT NOT NULL,model TEXT NOT NULL,effort TEXT NOT NULL,rev INTEGER NOT NULL,PRIMARY KEY(scope,role));
CREATE TABLE IF NOT EXISTS artifacts(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,meta TEXT NOT NULL,encoded TEXT NOT NULL,UNIQUE(project,id));
CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,revision INTEGER NOT NULL DEFAULT 1,state TEXT NOT NULL DEFAULT 'open',intent TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS submissions(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,request TEXT NOT NULL REFERENCES requests,key TEXT NOT NULL,hash TEXT NOT NULL,content TEXT NOT NULL,UNIQUE(project,key));
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
CREATE TABLE IF NOT EXISTS blobs(project TEXT NOT NULL,digest TEXT NOT NULL,content TEXT NOT NULL,PRIMARY KEY(project,digest));

CREATE TABLE IF NOT EXISTS tools(version TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,name TEXT NOT NULL,source TEXT NOT NULL,export TEXT NOT NULL,contract TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'draft',runtime TEXT NOT NULL,evidence TEXT);
CREATE TABLE IF NOT EXISTS tool_tests(id TEXT PRIMARY KEY,project TEXT NOT NULL,version TEXT NOT NULL REFERENCES tools,request TEXT NOT NULL,expected TEXT NOT NULL,observed TEXT,outcome TEXT NOT NULL,receipt TEXT NOT NULL,provisional INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS tool_defaults(project TEXT NOT NULL,name TEXT NOT NULL,version TEXT NOT NULL REFERENCES tools,PRIMARY KEY(project,name));
CREATE TABLE IF NOT EXISTS inventory(snapshot TEXT NOT NULL,path TEXT NOT NULL,parent TEXT NOT NULL,kind TEXT NOT NULL,digest TEXT,bytes INTEGER NOT NULL,PRIMARY KEY(snapshot,path));
CREATE TABLE IF NOT EXISTS dictionaries(id TEXT PRIMARY KEY,owner TEXT,name TEXT NOT NULL,archived INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS dictionary_links(project TEXT NOT NULL REFERENCES projects,dictionary TEXT NOT NULL REFERENCES dictionaries,enabled INTEGER NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(project,dictionary));
CREATE TABLE IF NOT EXISTS entries(id TEXT NOT NULL,revision INTEGER NOT NULL,dictionary TEXT NOT NULL REFERENCES dictionaries,name TEXT NOT NULL,kind TEXT NOT NULL,definition TEXT NOT NULL,source TEXT NOT NULL,human INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(id,revision));
CREATE TABLE IF NOT EXISTS skills(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,name TEXT NOT NULL,digest TEXT NOT NULL,description TEXT NOT NULL,body TEXT NOT NULL,resources TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS proposals(id TEXT PRIMARY KEY,project TEXT NOT NULL,entry TEXT,base_revision INTEGER,proposed TEXT NOT NULL,source TEXT NOT NULL,status TEXT NOT NULL);
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
CREATE TABLE IF NOT EXISTS git_mutations(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,operation TEXT NOT NULL,request TEXT NOT NULL,digest TEXT NOT NULL,state TEXT NOT NULL,grant_id TEXT,result TEXT,error TEXT);
CREATE TABLE IF NOT EXISTS git_grants(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,mutation TEXT NOT NULL REFERENCES git_mutations,digest TEXT NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS file_effects(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,operation TEXT NOT NULL,request TEXT NOT NULL,digest TEXT NOT NULL,state TEXT NOT NULL,grant_id TEXT,result TEXT,error TEXT);
CREATE TABLE IF NOT EXISTS file_grants(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,effect TEXT NOT NULL REFERENCES file_effects,digest TEXT NOT NULL,destructive INTEGER NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS patch_stages(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,request TEXT NOT NULL,digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS connectors(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,name TEXT NOT NULL,definition TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS network_effects(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects,connector TEXT NOT NULL REFERENCES connectors,arguments TEXT NOT NULL,state TEXT NOT NULL,receipt TEXT);
