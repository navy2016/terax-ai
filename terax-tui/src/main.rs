use anyhow::{Context, Result};
use crossterm::{event::{self, Event, KeyCode, KeyEvent, KeyModifiers}, execute, terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}};
use ratatui::{backend::CrosstermBackend, layout::{Constraint, Direction, Layout, Rect}, style::{Color, Modifier, Style}, text::{Line, Span}, widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap}, Frame, Terminal};
use std::{env, fs, io::{self, Stdout}, path::{Path, PathBuf}, process::Command, sync::mpsc::{self, Receiver}, thread, time::Duration};
use terax_core::{ai::{self, ChatMessage}, config::Config, git::GitEntry, pty::PtySession, tools::{ToolRequest, ToolRisk}};
use walkdir::WalkDir;

#[derive(Clone, Copy, Debug, Eq, PartialEq)] enum Focus { Files, Git, Terminal, Editor, Command, Chat, Log }
#[derive(Clone, Copy, Debug, Eq, PartialEq)] enum View { Workspace, Ai, Git, Terminal, Editor }
#[derive(Debug)] enum AiEvent { Done(String), EditProposal(String), BufferProposal(String), Error(String) }
#[derive(Debug, Clone)] struct PendingEdit { path: Option<PathBuf>, row: usize, old: String, new: String }
#[derive(Debug, Clone)] struct PendingBufferEdit { path: Option<PathBuf>, old_lines: Vec<String>, new_lines: Vec<String> }

struct App {
    cwd: PathBuf, entries: Vec<PathBuf>, selected: usize, preview: String,
    command: String, logs: Vec<String>, focus: Focus, view: View, should_quit: bool,
    chat: Vec<ChatMessage>, ai_busy: bool, ai_rx: Option<Receiver<AiEvent>>, status: String, git_entries: Vec<GitEntry>, git_selected: usize, git_diff: String, pty: Option<PtySession>, terminal_input: String, terminal_lines: Vec<String>, editor_path: Option<PathBuf>, editor_lines: Vec<String>, editor_row: usize, editor_col: usize, editor_insert: bool, editor_dirty: bool, pending_tool: Option<ToolRequest>, pending_edit: Option<PendingEdit>, pending_buffer_edit: Option<PendingBufferEdit>,
}
impl App {
    fn new() -> Result<Self> { let cwd=env::current_dir()?; let mut a=Self{cwd,entries:vec![],selected:0,preview:String::new(),command:String::new(),logs:vec!["Terax TUI v0.8".into(),"Ctrl+A AI | Ctrl+G Git | Ctrl+T Term | Ctrl+E Editor | ai-edit-line/buffer | a/n".into()],focus:Focus::Files,view:View::Workspace,should_quit:false,chat:vec![ai::system("You are Terax TUI assistant. Be concise, practical, and code-aware.")],ai_busy:false,ai_rx:None,status:"AI idle".into(),git_entries:vec![],git_selected:0,git_diff:String::new(),pty:None,terminal_input:String::new(),terminal_lines:vec!["Terminal not started. Press Ctrl+T.".into()],editor_path:None,editor_lines:vec!["No file open. Select a file and press Enter, or Ctrl+E.".into()],editor_row:0,editor_col:0,editor_insert:false,editor_dirty:false,pending_tool:None,pending_edit:None,pending_buffer_edit:None}; a.refresh()?; a.refresh_git(); Ok(a) }
    fn refresh(&mut self)->Result<()> { self.entries.clear(); self.entries.push(self.cwd.join("..")); let mut dirs=vec![]; let mut files=vec![]; for e in fs::read_dir(&self.cwd)? { let p=e?.path(); let name=p.file_name().and_then(|s|s.to_str()).unwrap_or(""); if name.starts_with('.') && name != ".github" {continue} if p.is_dir(){dirs.push(p)}else{files.push(p)} } dirs.sort(); files.sort(); self.entries.extend(dirs); self.entries.extend(files); self.selected=self.selected.min(self.entries.len().saturating_sub(1)); self.update_preview(); Ok(()) }
    fn selected_path(&self)->Option<&Path>{self.entries.get(self.selected).map(|p|p.as_path())}
    fn update_preview(&mut self){ let Some(p)=self.selected_path() else {self.preview.clear(); return}; if p.is_dir(){let c=WalkDir::new(p).max_depth(2).into_iter().filter_map(Result::ok).count(); self.preview=format!("Directory: {}\nEntries within depth 2: {}",p.display(),c); return} match terax_core::fs::read_text_limited(p,256*1024){Ok(s)=>self.preview=s.lines().take(300).collect::<Vec<_>>().join("\n"),Err(e)=>self.preview=format!("Preview error: {e:#}")} }
    fn open_selected(&mut self)->Result<()> { let Some(p)=self.selected_path().map(Path::to_path_buf) else{return Ok(())}; if p.is_dir(){self.cwd=terax_core::fs::canonical_or_original(p); self.selected=0; self.refresh()?; self.logs.push(format!("cwd -> {}",self.cwd.display()))} else {self.update_preview(); self.logs.push(format!("preview -> {}",p.display()))} Ok(()) }
    fn poll_ai(&mut self){ if let Some(rx)=self.ai_rx.take(){ match rx.try_recv(){ Ok(AiEvent::Done(ans))=>{self.ai_busy=false; self.status="AI done".into(); self.chat.push(ai::assistant(ans.clone())); self.logs.push("AI done".into());}, Ok(AiEvent::EditProposal(new_line))=>{self.ai_busy=false; self.status="AI edit proposed".into(); self.create_pending_line_edit(new_line);}, Ok(AiEvent::BufferProposal(new_text))=>{self.ai_busy=false; self.status="AI buffer edit proposed".into(); self.create_pending_buffer_edit(new_text);}, Ok(AiEvent::Error(e))=>{self.ai_busy=false; self.status="AI error".into(); self.logs.push(format!("AI error: {e}")); self.chat.push(ai::assistant(format!("Error: {e}")));}, Err(mpsc::TryRecvError::Empty)=>{ self.ai_rx=Some(rx); }, Err(mpsc::TryRecvError::Disconnected)=>{self.ai_busy=false; self.status="AI disconnected".into();} } } }
    fn context_for(&self, prompt:&str)->String{ let mut out=String::new(); out.push_str(prompt); if let Some(p)=self.selected_path(){ if p.is_file(){ if let Ok(cfg)=Config::load_default(){ let max=cfg.max_context_bytes(); if let Ok(txt)=terax_core::fs::read_text_limited(p,max){ out.push_str(&format!("\n\n[Current file: {}]\n```\n{}\n```",p.display(),txt)); } } } } out }
    fn ask_ai(&mut self, prompt:String){ if self.ai_busy { self.logs.push("AI busy".into()); return; } let mut msgs=self.chat.clone(); msgs.push(ai::user(self.context_for(&prompt))); self.chat.push(ai::user(prompt)); let (tx,rx)=mpsc::channel(); self.ai_rx=Some(rx); self.ai_busy=true; self.status="AI thinking...".into(); thread::spawn(move||{ let _=tx.send(match ai::chat(msgs){Ok(a)=>AiEvent::Done(a),Err(e)=>AiEvent::Error(format!("{e:#}"))}); }); }





    fn ask_ai_edit_line(&mut self, instruction: String){
        if self.ai_busy { self.logs.push("AI busy".into()); return; }
        let old=self.editor_lines.get(self.editor_row).cloned().unwrap_or_default();
        let path=self.editor_path.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||"<unsaved>".into());
        let prompt=format!("You are editing one line in a code editor. Return ONLY the replacement line, no markdown, no explanation.\nFile: {}\nLine {}: {}\nInstruction: {}", path, self.editor_row+1, old, instruction);
        let msgs=vec![ai::system("Return only the replacement line. No markdown."), ai::user(prompt)];
        let (tx,rx)=mpsc::channel(); self.ai_rx=Some(rx); self.ai_busy=true; self.status="AI editing line...".into();
        thread::spawn(move||{ let _=tx.send(match ai::chat(msgs){Ok(a)=>AiEvent::EditProposal(a.lines().next().unwrap_or("").to_string()),Err(e)=>AiEvent::Error(format!("{e:#}"))}); });
    }

    fn ask_ai_edit_buffer(&mut self, instruction: String){
        if self.ai_busy { self.logs.push("AI busy".into()); return; }
        let path=self.editor_path.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||"<unsaved>".into());
        let content=self.editor_lines.join("\n");
        let prompt=format!("You are editing a file in a code editor. Return ONLY the full replacement file content, no markdown fences, no explanation.\nFile: {}\nInstruction: {}\n\nCurrent content:\n{}", path, instruction, content);
        let msgs=vec![ai::system("Return only full replacement file content. No markdown fences. No explanation."), ai::user(prompt)];
        let (tx,rx)=mpsc::channel(); self.ai_rx=Some(rx); self.ai_busy=true; self.status="AI editing buffer...".into();
        thread::spawn(move||{ let _=tx.send(match ai::chat(msgs){Ok(a)=>AiEvent::BufferProposal(strip_code_fence(a)),Err(e)=>AiEvent::Error(format!("{e:#}"))}); });
    }
    fn create_pending_buffer_edit(&mut self, new_text: String){
        let new_lines:Vec<String>=new_text.lines().map(|s|s.to_string()).collect();
        let old_count=self.editor_lines.len();
        let new_count=new_lines.len();
        self.pending_buffer_edit=Some(PendingBufferEdit{path:self.editor_path.clone(), old_lines:self.editor_lines.clone(), new_lines});
        self.logs.push(format!("PENDING BUFFER EDIT: {} lines -> {} lines", old_count, new_count));
        self.logs.push("Apply buffer edit? a / n".into());
    }
    fn apply_pending_buffer_edit(&mut self){
        let Some(edit)=self.pending_buffer_edit.take() else { return; };
        self.editor_lines=if edit.new_lines.is_empty(){vec![String::new()]}else{edit.new_lines};
        self.editor_row=0; self.editor_col=0; self.editor_dirty=true;
        self.view=View::Editor; self.focus=Focus::Editor;
        self.logs.push("applied buffer edit proposal".into());
    }
    fn deny_pending_buffer_edit(&mut self){ if self.pending_buffer_edit.take().is_some(){ self.logs.push("denied buffer edit proposal".into()); } }

    fn create_pending_line_edit(&mut self, new_line: String){
        let old=self.editor_lines.get(self.editor_row).cloned().unwrap_or_default();
        self.pending_edit=Some(PendingEdit{path:self.editor_path.clone(), row:self.editor_row, old:old.clone(), new:new_line.clone()});
        self.logs.push(format!("PENDING EDIT line {}", self.editor_row+1));
        self.logs.push(format!("- {}", old));
        self.logs.push(format!("+ {}", new_line));
        self.logs.push("Apply edit? a / n".into());
    }
    fn apply_pending_edit(&mut self){
        let Some(edit)=self.pending_edit.take() else { return; };
        if edit.row < self.editor_lines.len(){
            self.editor_lines[edit.row]=edit.new;
            self.editor_row=edit.row;
            self.editor_col=self.editor_lines[edit.row].len();
            self.editor_dirty=true;
            self.view=View::Editor; self.focus=Focus::Editor;
            self.logs.push("applied edit proposal".into());
        }
    }
    fn deny_pending_edit(&mut self){ if self.pending_edit.take().is_some(){ self.logs.push("denied edit proposal".into()); } }

    fn set_pending(&mut self, req: ToolRequest){
        self.logs.push(format!("PENDING {:?}: {} [risk: {:?}]", req.kind, req.summary, req.risk));
        self.logs.push("Approve? y / n".into());
        self.pending_tool=Some(req);
    }
    fn approve_pending(&mut self){
        let Some(req)=self.pending_tool.take() else { return; };
        match req.kind {
            terax_core::tools::ToolKind::TerminalSend => {
                self.ensure_pty();
                if let Some(p)=self.pty.as_mut(){ let _=p.write(&(req.payload.clone()+"\n")); }
                self.logs.push(format!("approved tool: {}", req.summary));
            }
            _ => self.logs.push(format!("tool not executable yet: {:?}", req.kind)),
        }
    }
    fn deny_pending(&mut self){
        if let Some(req)=self.pending_tool.take(){ self.logs.push(format!("denied tool: {}", req.summary)); }
    }
    fn ask_terminal_tail(&mut self){
        let tail=self.terminal_tail(80);
        self.ask_ai(format!("Analyze this terminal output. Explain errors and next steps.\n\n[terminal tail]\n{}", tail));
    }
    fn ask_git_status(&mut self){
        let status=terax_core::git::status(&self.cwd).unwrap_or_else(|e|format!("git status error: {e:#}"));
        self.ask_ai(format!("Analyze this git status. Summarize changes and suggest next actions.\n\n{}", status));
    }

    fn open_editor(&mut self, path: PathBuf){
        if path.is_dir(){ return; }
        match terax_core::fs::read_text_limited(&path, 512*1024){
            Ok(txt)=>{ self.editor_path=Some(path.clone()); self.editor_lines=txt.lines().map(|s|s.to_string()).collect(); if self.editor_lines.is_empty(){self.editor_lines.push(String::new());} self.editor_row=0; self.editor_col=0; self.editor_dirty=false; self.view=View::Editor; self.focus=Focus::Editor; self.logs.push(format!("editor open -> {}", path.display())); },
            Err(e)=>self.logs.push(format!("editor open error: {e:#}")),
        }
    }
    fn save_editor(&mut self){
        let Some(path)=self.editor_path.clone() else { self.logs.push("editor: no file".into()); return; };
        let data=self.editor_lines.join("\n");
        match std::fs::write(&path, data){ Ok(_)=>{self.editor_dirty=false; self.logs.push(format!("saved {}", path.display()));}, Err(e)=>self.logs.push(format!("save error: {e:#}")) }
    }
    fn editor_move_up(&mut self){ self.editor_row=self.editor_row.saturating_sub(1); self.editor_col=self.editor_col.min(self.editor_lines.get(self.editor_row).map(|l|l.len()).unwrap_or(0)); }
    fn editor_move_down(&mut self){ if self.editor_row+1<self.editor_lines.len(){self.editor_row+=1;} self.editor_col=self.editor_col.min(self.editor_lines.get(self.editor_row).map(|l|l.len()).unwrap_or(0)); }
    fn editor_move_left(&mut self){ self.editor_col=self.editor_col.saturating_sub(1); }
    fn editor_move_right(&mut self){ let len=self.editor_lines.get(self.editor_row).map(|l|l.len()).unwrap_or(0); if self.editor_col<len{self.editor_col+=1;} }
    fn editor_insert_char(&mut self, c: char){ if self.editor_lines.is_empty(){self.editor_lines.push(String::new());} if let Some(line)=self.editor_lines.get_mut(self.editor_row){ let idx=self.editor_col.min(line.len()); line.insert(idx,c); self.editor_col=idx+1; self.editor_dirty=true; } }
    fn editor_backspace(&mut self){ if self.editor_lines.is_empty(){return;} if self.editor_col>0{ if let Some(line)=self.editor_lines.get_mut(self.editor_row){ let idx=self.editor_col-1; if idx<line.len(){line.remove(idx); self.editor_col-=1; self.editor_dirty=true;} } } else if self.editor_row>0 { let cur=self.editor_lines.remove(self.editor_row); self.editor_row-=1; self.editor_col=self.editor_lines[self.editor_row].len(); self.editor_lines[self.editor_row].push_str(&cur); self.editor_dirty=true; } }
    fn editor_newline(&mut self){ if self.editor_lines.is_empty(){self.editor_lines.push(String::new());} let tail={ let line=&mut self.editor_lines[self.editor_row]; let idx=self.editor_col.min(line.len()); line.split_off(idx) }; self.editor_row+=1; self.editor_col=0; self.editor_lines.insert(self.editor_row, tail); self.editor_dirty=true; }

    fn ensure_pty(&mut self){
        if self.pty.is_none(){
            match PtySession::spawn(Some("sh".into()), 100, 24, Some(self.cwd.clone())){
                Ok(p)=>{self.pty=Some(p); self.terminal_lines.clear(); self.logs.push("PTY shell started".into());},
                Err(e)=>self.logs.push(format!("PTY start error: {e:#}")),
            }
        }
    }
    fn poll_pty(&mut self){
        if let Some(p)=self.pty.as_mut(){
            for chunk in p.try_read_all(){
                for line in chunk.replace('\r',"").split('\n'){
                    if !line.is_empty(){ self.terminal_lines.push(line.to_string()); }
                }
            }
            if self.terminal_lines.len()>1000{ let n=self.terminal_lines.len()-1000; self.terminal_lines.drain(0..n); }
        }
    }
    fn terminal_tail(&self, n:usize)->String{
        let start=self.terminal_lines.len().saturating_sub(n);
        self.terminal_lines[start..].join("\n")
    }
    fn terminal_send(&mut self){
        self.ensure_pty();
        let input=self.terminal_input.clone();
        if let Some(p)=self.pty.as_mut(){ let _=p.write(&(input.clone()+"\n")); }
        self.terminal_input.clear();
    }

    fn refresh_git(&mut self){
        match terax_core::git::status_entries(&self.cwd){
            Ok(v)=>{self.git_entries=v; if self.git_selected>=self.git_entries.len(){self.git_selected=self.git_entries.len().saturating_sub(1);} self.update_git_diff();},
            Err(e)=>{self.git_entries.clear(); self.git_diff=format!("git status error: {e:#}");}
        }
    }
    fn selected_git(&self)->Option<&GitEntry>{self.git_entries.get(self.git_selected)}
    fn update_git_diff(&mut self){
        let Some(e)=self.selected_git().cloned() else { self.git_diff="No git changes".into(); return; };
        let unstaged=terax_core::git::diff_path(&self.cwd,&e.path).unwrap_or_default();
        let staged=terax_core::git::staged_diff_path(&self.cwd,&e.path).unwrap_or_default();
        self.git_diff=format!("status: {} {}\n\n[staged diff]\n{}\n\n[unstaged diff]\n{}",e.status,e.path,staged,unstaged);
    }
    fn toggle_stage(&mut self){
        let Some(e)=self.selected_git().cloned() else {return};
        let res=if e.staged{terax_core::git::unstage(&self.cwd,&e.path)}else{terax_core::git::stage(&self.cwd,&e.path)};
        match res{Ok(_)=>self.logs.push(format!("toggled stage: {}",e.path)),Err(err)=>self.logs.push(format!("stage error: {err:#}"))}
        self.refresh_git();
    }
    fn commit_message(&mut self){
        let staged=terax_core::git::staged_diff(&self.cwd).unwrap_or_else(|e|format!("staged diff error: {e:#}"));
        if staged.trim().is_empty(){ self.logs.push("No staged diff for commit message".into()); return; }
        self.ask_ai(format!("Generate a concise conventional commit message for this staged diff. Return only the commit message.\n\n{}",staged));
    }

    fn review_diff(&mut self){ let status=terax_core::git::status(&self.cwd).unwrap_or_else(|e|format!("git status error: {e:#}")); let diff=terax_core::git::diff(&self.cwd).unwrap_or_else(|e|format!("git diff error: {e:#}")); self.ask_ai(format!("Review this git status and diff. Focus on risks, bugs, and concise suggestions.\n\n[git status]\n{}\n\n[git diff]\n{}",status,diff)); }
    fn explain_current(&mut self){ if let Some(p)=self.selected_path(){ self.ask_ai(format!("Explain the current file: {}. Summarize purpose, structure, and likely risks.",p.display())) } else { self.logs.push("No file selected".into()) } }
    fn run_command(&mut self){ let cmd=self.command.trim().to_string(); if cmd.is_empty(){return} self.logs.push(format!("> {cmd}")); if cmd=="quit"||cmd=="exit"{self.should_quit=true;return} if cmd=="pwd"{self.logs.push(self.cwd.display().to_string()); self.command.clear(); return} if cmd=="/explain"{self.explain_current(); self.command.clear(); return} if cmd=="/review-diff"{self.review_diff(); self.command.clear(); return} if cmd=="/commit-message"{self.commit_message(); self.command.clear(); return} if cmd=="/terminal-tail"{self.ask_terminal_tail(); self.command.clear(); return} if cmd=="/git-status"{self.ask_git_status(); self.command.clear(); return} if let Some(inst)=cmd.strip_prefix("ai-edit-buffer "){self.ask_ai_edit_buffer(inst.trim().to_string()); self.command.clear(); return} if let Some(inst)=cmd.strip_prefix("ai-edit-line "){self.ask_ai_edit_line(inst.trim().to_string()); self.command.clear(); return} if let Some(p)=cmd.strip_prefix("ai "){self.ask_ai(p.trim().to_string()); self.command.clear(); return} if let Some(c)=cmd.strip_prefix("run "){self.set_pending(ToolRequest::terminal_send(c.trim())); self.command.clear(); return} if let Some(rest)=cmd.strip_prefix("cd "){let t=self.cwd.join(rest.trim()); if t.is_dir(){self.cwd=terax_core::fs::canonical_or_original(t); self.selected=0; let _=self.refresh();} self.command.clear(); return} let output=Command::new("sh").arg("-lc").arg(&cmd).current_dir(&self.cwd).output(); match output{Ok(o)=>{for l in String::from_utf8_lossy(&o.stdout).lines().take(120){self.logs.push(l.into())} for l in String::from_utf8_lossy(&o.stderr).lines().take(120){self.logs.push(format!("ERR: {l}"))} self.logs.push(format!("exit: {}",o.status));},Err(e)=>self.logs.push(format!("command failed: {e}"))} self.command.clear(); }
    fn on_key(&mut self,key:KeyEvent)->Result<()> { if key.modifiers.contains(KeyModifiers::CONTROL)&&key.code==KeyCode::Char('c'){self.should_quit=true; return Ok(())} if key.modifiers.contains(KeyModifiers::CONTROL)&&key.code==KeyCode::Char('a'){self.view=if self.view==View::Ai{View::Workspace}else{View::Ai}; self.focus=Focus::Command; return Ok(())} if key.modifiers.contains(KeyModifiers::CONTROL)&&key.code==KeyCode::Char('g'){self.view=if self.view==View::Git{View::Workspace}else{View::Git}; self.focus=Focus::Git; self.refresh_git(); return Ok(())} if key.modifiers.contains(KeyModifiers::CONTROL)&&key.code==KeyCode::Char('t'){self.view=if self.view==View::Terminal{View::Workspace}else{View::Terminal}; self.focus=Focus::Terminal; self.ensure_pty(); return Ok(())} if key.modifiers.contains(KeyModifiers::CONTROL)&&key.code==KeyCode::Char('e'){self.view=if self.view==View::Editor{View::Workspace}else{View::Editor}; self.focus=Focus::Editor; return Ok(())} if key.modifiers.contains(KeyModifiers::CONTROL)&&key.code==KeyCode::Char('l') && self.focus==Focus::Terminal {self.terminal_lines.clear(); return Ok(())} if key.modifiers.contains(KeyModifiers::CONTROL)&&key.code==KeyCode::Char('s') && self.focus==Focus::Editor {self.save_editor(); return Ok(())} if self.pending_buffer_edit.is_some(){ match key.code { KeyCode::Char('a')=>{self.apply_pending_buffer_edit(); return Ok(())}, KeyCode::Char('n')|KeyCode::Esc=>{self.deny_pending_buffer_edit(); return Ok(())}, _=>{} } } if self.pending_edit.is_some(){ match key.code { KeyCode::Char('a')=>{self.apply_pending_edit(); return Ok(())}, KeyCode::Char('n')|KeyCode::Esc=>{self.deny_pending_edit(); return Ok(())}, _=>{} } } if self.pending_tool.is_some(){ match key.code { KeyCode::Char('y')=>{self.approve_pending(); return Ok(())}, KeyCode::Char('n')|KeyCode::Esc=>{self.deny_pending(); return Ok(())}, _=>{} } } match self.focus{ Focus::Files=>match key.code{KeyCode::Char('q')=>self.should_quit=true,KeyCode::Tab=>self.focus=Focus::Command,KeyCode::Char(':')=>self.focus=Focus::Command,KeyCode::Char('r')=>self.refresh()?,KeyCode::Up=>{self.selected=self.selected.saturating_sub(1);self.update_preview()},KeyCode::Down=>{if self.selected+1<self.entries.len(){self.selected+=1} self.update_preview()},KeyCode::Enter=>{ if let Some(p)=self.selected_path().map(Path::to_path_buf){ if p.is_file(){self.open_editor(p)} else {self.open_selected()?} } },_=>{}}, Focus::Git=>match key.code{KeyCode::Char('q')=>self.should_quit=true,KeyCode::Tab|KeyCode::Esc=>self.focus=Focus::Command,KeyCode::Char('r')=>self.refresh_git(),KeyCode::Char('s')=>self.toggle_stage(),KeyCode::Up=>{self.git_selected=self.git_selected.saturating_sub(1);self.update_git_diff()},KeyCode::Down=>{if self.git_selected+1<self.git_entries.len(){self.git_selected+=1} self.update_git_diff()},_=>{}}, Focus::Terminal=>match key.code{KeyCode::Esc=>self.focus=Focus::Files,KeyCode::Tab=>self.focus=Focus::Command,KeyCode::Enter=>self.terminal_send(),KeyCode::Backspace=>{self.terminal_input.pop();},KeyCode::Char(c)=>self.terminal_input.push(c),_=>{}}, Focus::Editor=>match key.code{KeyCode::Esc=>{self.editor_insert=false; self.focus=Focus::Files},KeyCode::Tab=>self.focus=Focus::Command,KeyCode::Up=>self.editor_move_up(),KeyCode::Down=>self.editor_move_down(),KeyCode::Left=>self.editor_move_left(),KeyCode::Right=>self.editor_move_right(),KeyCode::Enter=>{if self.editor_insert{self.editor_newline()}},KeyCode::Backspace=>{if self.editor_insert{self.editor_backspace()}},KeyCode::Char('i') if !self.editor_insert=>self.editor_insert=true,KeyCode::Char(c)=>{if self.editor_insert{self.editor_insert_char(c)}},_=>{}}, Focus::Command=>match key.code{KeyCode::Esc=>self.focus=Focus::Files,KeyCode::Tab=>self.focus=if self.view==View::Ai{Focus::Chat}else{Focus::Log},KeyCode::Enter=>self.run_command(),KeyCode::Backspace=>{self.command.pop();},KeyCode::Char(c)=>self.command.push(c),_=>{}}, Focus::Chat|Focus::Log=>match key.code{KeyCode::Tab|KeyCode::Esc=>self.focus=Focus::Files,KeyCode::Char('q')=>self.should_quit=true,_=>{}} } Ok(()) }
}

fn strip_code_fence(s:String)->String{
    let t=s.trim();
    if t.starts_with("```"){
        let mut lines=t.lines().collect::<Vec<_>>();
        if !lines.is_empty(){lines.remove(0);} 
        if lines.last().map(|l|l.trim()=="```").unwrap_or(false){lines.pop();}
        lines.join("\n")
    }else{s}
}

fn focus_style(active:bool)->Style{if active{Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)}else{Style::default()}}
fn ui(f:&mut Frame, app:&mut App){ app.poll_ai(); app.poll_pty(); let root=Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(8),Constraint::Length(3),Constraint::Length(7)]).split(f.area()); let main=Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(34),Constraint::Percentage(66)]).split(root[0]); render_files(f,app,main[0]); if app.view==View::Ai{render_chat(f,app,main[1])}else if app.view==View::Git{render_git(f,app,main[1])}else if app.view==View::Terminal{render_terminal(f,app,main[1])}else if app.view==View::Editor{render_editor(f,app,main[1])}else{render_preview(f,app,main[1])} render_command(f,app,root[1]); render_log(f,app,root[2]); }
fn render_files(f:&mut Frame, app:&mut App, area:Rect){ let items:Vec<ListItem>=app.entries.iter().map(|p|{let n=p.file_name().and_then(|s|s.to_str()).unwrap_or(".."); let icon=if p.is_dir(){"▸"}else{" "}; ListItem::new(Line::from(vec![Span::raw(icon),Span::raw(" "),Span::raw(n.to_string())]))}).collect(); let mut st=ListState::default(); st.select(Some(app.selected)); let list=List::new(items).block(Block::default().title(format!(" Files: {} ",app.cwd.display())).borders(Borders::ALL).border_style(focus_style(app.focus==Focus::Files))).highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD)); f.render_stateful_widget(list,area,&mut st); }



fn render_editor(f:&mut Frame, app:&mut App, area:Rect){
    let h=area.height.saturating_sub(2) as usize;
    let start=app.editor_row.saturating_sub(h/2);
    let mut out=String::new();
    for (i,line) in app.editor_lines.iter().enumerate().skip(start).take(h){
        let mark=if i==app.editor_row{">"}else{" "};
        out.push_str(&format!("{}{:4} {}\n", mark, i+1, line));
    }
    let title=match &app.editor_path{Some(p)=>format!(" Editor {} [{}] row:{} col:{} {} ",p.display(), if app.editor_insert{"INSERT"}else{"NORMAL"}, app.editor_row+1, app.editor_col+1, if app.editor_dirty{"*"}else{""}),None=>" Editor ".into()};
    f.render_widget(Paragraph::new(out).block(Block::default().title(title).borders(Borders::ALL).border_style(focus_style(app.focus==Focus::Editor))).wrap(Wrap{trim:false}),area);
}

fn render_terminal(f:&mut Frame, app:&mut App, area:Rect){
    let chunks=Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(5),Constraint::Length(3)]).split(area);
    let h=chunks[0].height.saturating_sub(2) as usize;
    let start=app.terminal_lines.len().saturating_sub(h);
    let text=app.terminal_lines[start..].join("\n");
    f.render_widget(Paragraph::new(text).block(Block::default().title(" Terminal [Ctrl+L clear] ").borders(Borders::ALL).border_style(focus_style(app.focus==Focus::Terminal))).wrap(Wrap{trim:false}),chunks[0]);
    f.render_widget(Paragraph::new(format!("$ {}",app.terminal_input)).block(Block::default().title(" Shell input ").borders(Borders::ALL)),chunks[1]);
}

fn render_git(f:&mut Frame, app:&mut App, area:Rect){
    let chunks=Layout::default().direction(Direction::Vertical).constraints([Constraint::Length((area.height/3).max(5)),Constraint::Min(5)]).split(area);
    let items:Vec<ListItem>=app.git_entries.iter().map(|e|ListItem::new(format!("{} {}",e.status,e.path))).collect();
    let mut st=ListState::default(); st.select(Some(app.git_selected));
    let list=List::new(items).block(Block::default().title(" Git status [s stage/unstage r refresh] ").borders(Borders::ALL).border_style(focus_style(app.focus==Focus::Git))).highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(list,chunks[0],&mut st);
    f.render_widget(Paragraph::new(app.git_diff.clone()).block(Block::default().title(" Diff ").borders(Borders::ALL)).wrap(Wrap{trim:false}),chunks[1]);
}

fn render_preview(f:&mut Frame, app:&App, area:Rect){ f.render_widget(Paragraph::new(app.preview.clone()).block(Block::default().title(" Preview ").borders(Borders::ALL)).wrap(Wrap{trim:false}),area)}
fn render_chat(f:&mut Frame, app:&App, area:Rect){ let mut s=String::new(); for m in app.chat.iter().filter(|m|m.role!="system").rev().take(12).collect::<Vec<_>>().into_iter().rev(){s.push_str(&format!("{}:\n{}\n\n",m.role,m.content));} f.render_widget(Paragraph::new(s).block(Block::default().title(format!(" AI Chat [{}] ",app.status)).borders(Borders::ALL).border_style(focus_style(app.focus==Focus::Chat))).wrap(Wrap{trim:false}),area)}
fn render_command(f:&mut Frame, app:&App, area:Rect){ f.render_widget(Paragraph::new(format!(":{}",app.command)).block(Block::default().title(" Command ").borders(Borders::ALL).border_style(focus_style(app.focus==Focus::Command))),area)}
fn render_log(f:&mut Frame, app:&App, area:Rect){ let h=area.height.saturating_sub(2) as usize; let start=app.logs.len().saturating_sub(h); f.render_widget(Paragraph::new(app.logs[start..].join("\n")).block(Block::default().title(if app.pending_buffer_edit.is_some(){" Log [PENDING BUFFER EDIT: a/n] "}else if app.pending_edit.is_some(){" Log [PENDING EDIT: a/n] "}else if app.pending_tool.is_some(){" Log [PENDING TOOL: y/n] "}else{" Log "}).borders(Borders::ALL).border_style(focus_style(app.focus==Focus::Log))).wrap(Wrap{trim:false}),area)}
struct Guard; impl Drop for Guard{fn drop(&mut self){let _=disable_raw_mode(); let _=execute!(io::stdout(),LeaveAlternateScreen);}}
fn run()->Result<()> { enable_raw_mode()?; execute!(io::stdout(),EnterAlternateScreen)?; let _g=Guard; let mut terminal:Terminal<CrosstermBackend<Stdout>>=Terminal::new(CrosstermBackend::new(io::stdout()))?; let mut app=App::new()?; while !app.should_quit{terminal.draw(|f|ui(f,&mut app))?; if event::poll(Duration::from_millis(100))?{if let Event::Key(k)=event::read()?{app.on_key(k)?}}} Ok(()) }
fn main(){ if let Err(e)=run(){eprintln!("terax-tui error: {e:#}"); std::process::exit(1)} }
