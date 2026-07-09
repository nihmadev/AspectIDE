pub fn harness_alias_hint(normalized: &str) -> Option<&'static str> {
    Some(match normalized {
        "toolsearch" => {
            "There is no deferred tool loading in Aspect вЂ” every callable tool is already in this request's tools array. For web research use WebResearch or MultiWebResearch; WebFetch for a known URL; SemanticSearch/Grep for code."
        }
        "bash" | "exec" | "execcommand" | "runcommand" | "runterminalcmd" | "runshellcommand" => {
            "Use Shell (one single-line command; cmd.exe /C on Windows)."
        }
        "edit" | "multiedit" | "editfile" | "strreplaceeditor" | "applypatch" | "applydiff" => {
            "Use StrReplace per edit (several edits = parallel StrReplace calls); PatchEngine only when edits must apply atomically together."
        }
        "websearch" | "searchweb" | "googlesearch" | "google" | "search" => {
            "Use WebResearch (open web question) or MultiWebResearch (several facets in parallel); SemanticSearch/Grep for code."
        }
        "askuserquestion" | "askfollowupquestion" => "Use AskUser.",
        "readfile" | "openfile" | "cat" | "viewfile" | "view" => {
            "Use Read (source/text) or InspectFile (tables/PDF/Office/archives/media/binaries)."
        }
        "ls" | "listdir" | "listdirectory" | "listfiles" => {
            "Use Glob (e.g. pattern \"src/*\")."
        }
        "agent" | "subagent" | "spawnagent" | "dispatchagent" | "workflow" => {
            "Use Task to run an isolated subagent."
        }
        "writefile" | "createfile" | "notebookedit" => "Use Write.",
        "codebasesearch" => "Use SemanticSearch.",
        "fetch" | "curl" | "httpget" | "fetchurl" => "Use WebFetch with the exact URL.",
        "skill" | "useskillfile" => "Use ListSkills to discover skills, then UseSkill to run one.",
        "sendmessage" => "Use AgentMessage (post/read on the shared agent board).",
        _ => return None,
    })
}
