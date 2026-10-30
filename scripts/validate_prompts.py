"""Validate the NekoWriter prompt pack: TOML syntax, key uniqueness, tool schemas/examples,
role tool lists, and strict rendering of every template with sample contexts (Jinja2 as a
stand-in for MiniJinja).

Usage: python3 -I scripts/validate_prompts.py <repo-root> [<prompt-key> | all]
The optional second argument prints the rendered output of one key (or of every key).
Requires Python >= 3.11 and Jinja2. Keep the expected key list and the role tool lists in sync
with docs/spec/09-prompts.md §2 and docs/spec/05-tools.md §2.
"""
import sys, tomllib, pathlib, re, itertools
import jinja2

ROOT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")
P = ROOT / "prompts"

def load(name):
    with open(P / name, "rb") as f:
        return tomllib.load(f)

errors = []
templates = {}
meta = {}
for fname in ["system.toml", "tasks.toml", "blocks.toml", "shared.toml", "notices.toml", "results.toml"]:
    data = load(fname)
    for entry in data.get("prompt", []):
        k = entry["key"]
        if k in templates:
            errors.append(f"duplicate key {k}")
        for field in ["key", "kind", "agent", "vars", "title_en", "title_zh", "text"]:
            if field not in entry:
                errors.append(f"{k}: missing {field}")
        templates[k] = entry["text"]
        meta[k] = entry
variables = load("variables.toml")
tools = load("tools.toml")["tools"]
for name, t in tools.items():
    templates[f"tool.{name}"] = t["description"]

# ---------- expected keys from spec/09 §2 ----------
expected = """conception.system researcher.system planner.system writer.system reviewer.system archivist.system
conception.kickoff researcher.kickoff planner.kickoff writer.kickoff.chapter writer.kickoff.revise
writer.kickoff.rollover reviewer.kickoff reviewer.rereview archivist.kickoff.maintenance archivist.kickoff.seed
archivist.kickoff.resync block.settings block.canon_systems block.writing_skills block.memory block.recent_chapters
block.goals block.chapter_draft block.cast_index block.chapter_diff
block.anchor block.book_brief notice.user_feedback notice.user_message notice.user_edits notice.setting_edits
notice.completion_revoked notice.tool_not_allowed notice.output_truncated notice.web_content notice.nudge_generic
notice.research_budget planner.notice.outline_rejected planner.notice.nudge planner.notice.retrieval_budget
writer.notice.outline_accepted writer.notice.outline_rejected
writer.notice.revision_plan_accepted writer.notice.revision_plan_rejected writer.notice.granularity
writer.notice.nudge_planning writer.notice.nudge_drafting writer.notice.nudge_revision_planning
writer.notice.nudge_revising writer.notice.retrieval_budget writer.rules.granularity
result.error result.truncated result.entity_card result.entity_list result.relations result.timeline
result.foreshadowing_list result.search_hits result.knowledge_entry result.chapter_text result.draft_text
result.draft_edit result.writer_control result.submit_review result.canon_write result.finish_maintenance
result.ask_user result.request_review result.conception_write result.web_search result.web_fetch
result.knowledge_write result.finish_research shared.label shared.chapter_label""".split()
for k in expected:
    if k not in templates:
        errors.append(f"missing expected key {k}")
for k in meta:
    if k not in expected:
        errors.append(f"unexpected key {k} (add to spec/09)")

# ---------- role tool lists from spec/05 §2 ----------
roles = {
 "planner": "get_entity list_entities get_relations query_timeline list_foreshadowing search_canon search_knowledge read_knowledge_entry search_chapters read_chapter read_draft submit_outline",
 "writer": "get_entity list_entities get_relations query_timeline list_foreshadowing search_canon search_knowledge read_knowledge_entry search_chapters read_chapter submit_outline submit_revision_plan read_draft insert_text insert_paragraphs replace_text rewrite_paragraph delete_paragraphs finish_draft finish_revision",
 "reviewer": "get_entity list_entities get_relations query_timeline list_foreshadowing search_canon search_knowledge read_knowledge_entry search_chapters read_chapter submit_review",
 "archivist": "get_entity list_entities get_relations query_timeline list_foreshadowing search_canon read_chapter create_entity update_entity set_relation end_relation record_appearances add_timeline_event update_timeline_event add_memory_note retire_memory_note add_foreshadowing resolve_foreshadowing set_chapter_summary propose_goal_change propose_setting_change finish_maintenance",
 "conception": "ask_user web_search web_fetch write_setting_doc edit_setting_doc upsert_character remove_character reorder_characters set_research_topics request_review",
 "researcher": "web_search web_fetch search_knowledge read_knowledge_entry save_knowledge_entry update_knowledge_entry finish_research",
}
all_role_tools = set(itertools.chain.from_iterable(v.split() for v in roles.values()))
for t in all_role_tools:
    if t not in tools:
        errors.append(f"tool {t} used by a role but not defined")
for t in tools:
    if t not in all_role_tools:
        errors.append(f"tool {t} defined but not used by any role")

# ---------- minimal JSON Schema validator for the subset we use ----------
def validate(schema, value, path="$"):
    errs = []
    ty = schema.get("type")
    tymap = {"object": dict, "array": list, "string": str, "integer": int, "boolean": bool}
    if ty:
        py = tymap[ty]
        ok = isinstance(value, py) and not (ty == "integer" and isinstance(value, bool))
        if not ok:
            return [f"{path}: expected {ty}, got {type(value).__name__}"]
    if "enum" in schema and value not in schema["enum"]:
        errs.append(f"{path}: {value!r} not in enum")
    if ty == "object":
        props = schema.get("properties", {})
        for r in schema.get("required", []):
            if r not in value:
                errs.append(f"{path}: missing required {r}")
        for k, v in value.items():
            if k in props:
                errs += validate(props[k], v, f"{path}.{k}")
            elif schema.get("additionalProperties") is False:
                errs.append(f"{path}: unexpected property {k}")
            elif isinstance(schema.get("additionalProperties"), dict):
                errs += validate(schema["additionalProperties"], v, f"{path}.{k}")
    if ty == "array":
        if "minItems" in schema and len(value) < schema["minItems"]:
            errs.append(f"{path}: fewer than {schema['minItems']} items")
        if "maxItems" in schema and len(value) > schema["maxItems"]:
            errs.append(f"{path}: more than {schema['maxItems']} items")
        if "items" in schema:
            for i, v in enumerate(value):
                errs += validate(schema["items"], v, f"{path}[{i}]")
    if ty == "integer":
        if "minimum" in schema and value < schema["minimum"]:
            errs.append(f"{path}: below minimum")
        if "maximum" in schema and value > schema["maximum"]:
            errs.append(f"{path}: above maximum")
    return errs

def check_schema_shape(schema, path):
    errs = []
    if schema.get("type") == "object":
        if schema.get("additionalProperties") is not False and not isinstance(schema.get("additionalProperties"), dict):
            errs.append(f"{path}: object without additionalProperties=false")
        for r in schema.get("required", []):
            if r not in schema.get("properties", {}):
                errs.append(f"{path}: required {r} not in properties")
        for k, v in schema.get("properties", {}).items():
            if "description" not in v:
                errs.append(f"{path}.{k}: missing description")
            errs += check_schema_shape(v, f"{path}.{k}")
    if schema.get("type") == "array" and "items" in schema:
        errs += check_schema_shape(schema["items"], f"{path}[]")
    return errs

for name, t in tools.items():
    for f in ["description", "input_schema", "examples"]:
        if f not in t:
            errors.append(f"tool {name}: missing {f}")
    errors += check_schema_shape(t["input_schema"], f"tool {name}")
    if not t.get("examples"):
        errors.append(f"tool {name}: no examples")
    for i, ex in enumerate(t.get("examples", [])):
        for e in validate(t["input_schema"], ex):
            errors.append(f"tool {name} example {i}: {e}")

# ---------- rendering ----------
env = jinja2.Environment(loader=jinja2.DictLoader(templates), undefined=jinja2.StrictUndefined,
                         trim_blocks=True, lstrip_blocks=True, autoescape=False, keep_trailing_newline=False)

book = dict(title="青云劫", genre="仙侠", idea="少年身怀残破剑灵。", writing_language="zh-CN",
            writing_language_name="简体中文", unspaced=True)
track_main = dict(is_main=True, kind="main", title="", synopsis="", canon_mode="canon", anchor_kind="none", anchor_label="")
track_side = dict(is_main=False, kind="side", title="剑冢旧事", synopsis="师父年轻时的故事。", canon_mode="isolated",
                  anchor_kind="after_chapter", anchor_label="第12章《剑冢》")
track_prequel = dict(track_side, anchor_kind="before_start", anchor_label="", canon_mode="canon")
chapter = dict(id=45, number=13, label="第13章", title="夜雨", guidance="让苏婉第一次展露实力。", outline="## 场景一\n- 目标",
               reference_notes="- 林凡左臂有伤", target_min=3000, target_max=4000, word_count=2350, has_draft=True)
chapter_blank = dict(chapter, title="", guidance="", outline="", reference_notes="", has_draft=False, word_count=0)
settings = dict(outline="# 大纲\n正文……", world="# 世界观", protagonist="# 主角",
                characters=[dict(rank=1, name="林凡", role_label="主角", summary="外门弟子", profile="## 身份\n……"),
                            dict(rank=2, name="苏婉", role_label="", summary="", profile="")],
                scopes=[dict(name="仙侠设定", description="修炼体系参考"), dict(name="杂项", description="")])
settings_empty = dict(outline="", world="", protagonist="", characters=[], scopes=[])
memory = dict(is_first=False, window_mid=10,
              recent_summaries=[dict(label="第11章", title="剑鸣", summary="……"), dict(label="第12章", title="", summary="")],
              previous_tail=dict(label="第12章", text="雨落下来。"),
              protagonist=dict(id=12, name="林凡", status="dead", status_label="已死亡", state="身处剑冢", after_anchor=False, attributes=[dict(key="修为", value="筑基中期")]),
              recent_changes=[dict(chapter_label="第11章", target="林凡(#12)", note="修为：筑基初期→筑基中期")],
              appearances=[dict(id=12, name="林凡", count=2, chapters=[dict(label="第11章", role_label="视角"), dict(label="第12章", role_label="主要")])],
              absent_important=[dict(id=18, name="张三", last_label="第2章")],
              key_notes=[dict(id=3, kind_label="世界规则", content="筑基之前无法御剑。")],
              open_foreshadowing=[dict(id=9, title="黑衣人的剑穗", planted_label="第5章")],
              cast=[dict(id=12, name="林凡", kind_label="人物", summary="外门弟子"), dict(id=20, name="青云宗", kind_label="组织", summary="")],
              counts=dict(character=10, organization=3, location=5, item=4, other=1, event=30, fact=12, world_rule=6, foreshadowing_open=4))
memory_first = dict(memory, is_first=True, recent_summaries=[], previous_tail=None, protagonist=None, recent_changes=[],
                    appearances=[], absent_important=[], key_notes=[], open_foreshadowing=[], cast=[],
                    counts={k: 0 for k in memory["counts"]})
draft = dict(paragraphs=[dict(pid="p1", text="林凡抬起头。"), dict(pid="p2", text="雨更大了")], tail_open=True, word_count=12, paragraph_count=2)
draft_empty = dict(paragraphs=[], tail_open=False, word_count=0, paragraph_count=0)
skills = [dict(name="基础写作", category="写作", body="用动作、对话与细节呈现场景，少用概括与总结。")]
chapters = [dict(number=12, label="第12章", title="剑鸣", text="雨落下来。\n\n他握紧了旧剑。", partial=True)]
feedback = dict(text="结尾太仓促。", issues=[dict(severity_label="严重", category_label="连续性", pids=["p30", "p31"], quote="他拔出青冥剑", problem="剑已毁", suggestion="改用断剑")])
gran = {k: dict(key=k, label=k, revising=False, tail_open=False, last_pid="p9") for k in ["sentence", "paragraph", "free", "chapter"]}
gran_cases = list(gran.values()) + [dict(gran["paragraph"], tail_open=True), dict(gran["chapter"], revising=True)]
prior = dict(attempt=1, verdict_label="建议修改", summary="结尾与时间线矛盾。", feedback="请修正 p30。", comment="第二条判断有误：令牌在第30章已归还。")

def systems(**off):
    value = dict(entities=True, timeline=True, memory=True, foreshadowing=True, goals=True)
    value.update(off)
    value["enabled"] = [name for name in ["entities", "timeline", "memory", "foreshadowing"] if value[name]]
    return value

systems_all = systems()
systems_cases = [systems_all, systems(entities=False), systems(timeline=False), systems(memory=False),
                 systems(foreshadowing=False), systems(entities=False, timeline=False, memory=False, foreshadowing=False)]
goals_sample = dict(nodes=[
    dict(id=8, level="plan", parent_id=None, title="守住剑冢", purpose="让林凡在宗门倾覆前守住最后的传承。", chapters="第10–15章", overdue=False,
         entities=["苏婉(#21)"], events=["#3 宗门夜变"], foreshadowing=[]),
    dict(id=12, level="task", parent_id=8, title="找回断剑", purpose="用断剑证明林凡的继承者身份。", chapters="第13章", overdue=True,
         entities=[], events=[], foreshadowing=["#9 黑衣人的剑穗"]),
], omitted=2)
goals_empty = dict(nodes=[], omitted=0)

S = dict(book=book, track=track_main, chapter=chapter, settings=settings, memory=memory, draft=draft, feedback=feedback,
         skills=skills, chapters=chapters, systems=systems_all, goals=goals_sample,
         previous_outline="旧章纲", granularity=gran["paragraph"], phase="drafting",
         revision_plan="1. p12：删去重复描写", author_notes=["打斗写得更短。", "苏婉不要哭。"], prior=None,
         review=dict(comment="第二条判断有误。", attempt=1),
         research=dict(topic="宋代市井", requirements="茶馆布局", scope_name="宋代社会", scope_description="宋代社会生活",
                       language_name="简体中文", existing_titles=["宋代酒楼"], max_tool_calls=40, max_entries=25),
         conception=dict(title="青云劫", idea="少年身怀剑灵", genre="", writing_language_name="简体中文"),
         cast_index=[dict(id=12, name="林凡", kind_label="人物", status="active", status_label="活跃", aliases=["凡哥", "林师弟"], summary="外门弟子"),
                     dict(id=13, name="老剑", kind_label="物品", status="destroyed", status_label="已损毁", aliases=[], summary="")],
         diff=dict(changes=[dict(op="insert", pid="p9", after=None, old="", new="新段"), dict(op="delete", pid="p3", after=None, old="旧段", new=""),
                          dict(op="modify", pid="p4", after=None, old="原", new="新")]),
         text="请把打斗写得更短。", enum="entity_kind", value="character",
         track_kind="main", track_title="正文", number=12, title="剑冢", with_title=True)

variants = {
  "block.settings": [dict(), dict(settings=settings_empty, book=dict(book, genre=""))],
  "block.writing_skills": [dict()],
  "block.recent_chapters": [dict(), dict(chapters=[dict(number=12, label="第12章", title="", text="雨落下来。", partial=False)])],
  "block.memory": [dict(systems=s) for s in systems_cases] + [
      dict(memory=memory_first, chapter=chapter_blank), dict(memory=memory_first, chapter=dict(chapter_blank, number=1, label="第1章")),
      dict(memory=dict(memory, protagonist=dict(memory["protagonist"], after_anchor=True, status="active")))],
  "block.canon_systems": [dict(systems=s) for s in systems_cases],
  "block.goals": [dict(goals=goals_sample, systems=s) for s in systems_cases] + [dict(goals=goals_empty, systems=systems_all)],
  "block.chapter_draft": [dict(), dict(draft=draft_empty, chapter=chapter_blank)],
  "block.anchor": [dict(track=track_side), dict(track=track_prequel), dict(track=dict(track_side, anchor_kind="after_end", canon_mode="canon"))],
  "block.cast_index": [dict(), dict(cast_index=[])],
  "block.chapter_diff": [dict(), dict(diff=dict(changes=[]))],
  "planner.kickoff": [dict(), dict(chapter=dict(chapter_blank, outline="## 旧细纲", reference_notes="")),
                      dict(track=track_side, chapter=dict(chapter_blank))],
  "writer.kickoff.chapter": [dict(feedback=None), dict(feedback=None, chapter=dict(chapter_blank, has_draft=False)),
                             dict(), dict(track=track_side, chapter=dict(chapter_blank, has_draft=False), feedback=None)],
  "writer.kickoff.revise": [dict(), dict(track=track_side, chapter=chapter_blank)],
  "writer.kickoff.rollover": [dict(), dict(feedback=None, chapter=chapter_blank),
                              dict(phase="revising", granularity=dict(gran["sentence"], revising=True)),
                              dict(phase="revising", granularity=dict(gran["chapter"], revising=True), revision_plan="", author_notes=[], chapter=chapter_blank,
                                   feedback=dict(text="改短一些", issues=[]))],
  "reviewer.kickoff": [dict(), dict(track=track_side, chapter=chapter_blank), dict(prior=prior)],
  "archivist.kickoff.maintenance": [dict(systems=s, goals=g) for s in systems_cases for g in [goals_empty, goals_sample]] + [dict(track=track_side)],
  "archivist.kickoff.seed": [dict(systems=s) for s in systems_cases],
  "archivist.kickoff.resync": [dict(systems=s) for s in systems_cases] + [dict(track=track_side)],
  "researcher.kickoff": [dict(), dict(research=dict(S["research"], requirements="", scope_description="", existing_titles=[]))],
  "conception.kickoff": [dict(), dict(conception=dict(S["conception"], genre="仙侠"))],
  "writer.rules.granularity": [dict(granularity=g) for g in gran_cases],
  "writer.notice.outline_accepted": [dict(granularity=g) for g in gran_cases],
  "writer.notice.revision_plan_accepted": [dict(granularity=dict(g, revising=True)) for g in gran_cases],
  "writer.notice.granularity": [dict(granularity=g) for g in gran_cases],
  "writer.notice.nudge_drafting": [dict(granularity=g) for g in gran_cases],
  "writer.notice.retrieval_budget": [dict(used=30)],
  "planner.notice.retrieval_budget": [dict(used=30)],
  "notice.user_edits": [dict(compact=False, counts=dict(modified=1, deleted=1, inserted=1),
                              modified=[dict(pid="p2", old="雨", new="雨更大了")], deleted=[dict(pid="p3", old="风")],
                              inserted=[dict(pid="p9", after=None, text="新"), dict(pid="p10", after="p9", text="又新")],
                              tail_changed=True, tail_open=False, outline_changed=False, outline="", notes_changed=False, reference_notes=""),
                         dict(compact=True, counts=dict(modified=5, deleted=2, inserted=9), modified=[], deleted=[], inserted=[], tail_changed=False, tail_open=False,
                              outline_changed=True, outline="## 场景一（作者改）", notes_changed=True, reference_notes="- 苏婉不哭"),
                         dict(compact=False, counts=dict(modified=0, deleted=0, inserted=0), modified=[], deleted=[], inserted=[], tail_changed=False, tail_open=False,
                              outline_changed=True, outline="## 场景一（作者改）", notes_changed=False, reference_notes="")],
  "notice.setting_edits": [dict(items=[dict(doc_label="世界观", kind="doc", full_text="# 世界观\n新", lines=[], characters=[], topics=[]),
                                       dict(doc_label="故事大纲", kind="doc", full_text="", lines=[dict(op="+", text="新增行"), dict(op="-", text="删除行")], characters=[], topics=[]),
                                       dict(doc_label="人物设定", kind="characters", full_text="", lines=[], topics=[],
                                            characters=[dict(c, changed_profile=("## 身份\n（作者改）" if c["rank"] == 1 else "")) for c in S["settings"]["characters"]]),
                                       dict(doc_label="资料收集主题", kind="topics", full_text="", lines=[], characters=[], topics=[dict(topic="宋代", scope="宋代社会", requirements="")])])],
  "notice.tool_not_allowed": [dict(tool="get_entity", phase_label="正文阶段", allowed=["insert_text", "read_draft"]),
                              dict(tool="record_appearances", phase_label="本任务", allowed=["create_entity", "finish_maintenance"])],
  "notice.web_content": [dict(body="外部内容")],
  "notice.research_budget": [dict(used_calls=40, max_calls=40, entries=10, max_entries=25)],
  # one case per label enum the code uses (spec/09 §1); a missing map would fail at runtime under strict undefined
  "shared.label": [dict(enum=e, value=v) for e, v in [
      ("entity_kind", "character"), ("entity_status", "dead"), ("appearance_role", "pov"), ("note_kind", "world_rule"),
      ("foreshadowing_status", "open"), ("relation_status", "ended"), ("setting_doc", "characters"),
      ("granularity", "free"), ("phase", "drafting"), ("phase", "none"), ("severity", "major"), ("category", "prose"),
      ("verdict", "revise"), ("target_type", "goal"), ("goal_level", "plan"), ("search_kind", "summary"),
      ("language", "zh-CN"), ("language", "xx")]],
  "shared.chapter_label": [dict(), dict(track_kind="side", track_title="剑冢旧事", number=2, title="", with_title=True), dict(with_title=False)],
  "result.truncated": [dict(next_offset=6000), dict(next_offset=None)],
  "result.entity_card": [dict(e=dict(id=12, name="林凡", kind_label="人物", is_protagonist=True, importance=5, status_label="活跃", aliases=["凡哥", "林师弟"],
                                     summary="外门弟子", state="身处剑冢", state_after_anchor=True, attributes=[dict(key="修为", value="筑基中期")], description="……"),
                              relations=[dict(subject="林凡(#12)", object="张三(#15)", kind="师兄", ended=False, until_label="", description="同门", since_label="第3章"),
                                         dict(subject="林凡(#12)", object="王五(#16)", kind="仇敌", ended=True, until_label="第9章", description="", since_label="")],
                              changes=[dict(chapter_label="第11章", note="突破", after_anchor=True)],
                              appearances=dict(first_label="第1章", last_label="第12章", window_size=10, window_count=8),
                              design=dict(role_label="主角", summary="……")),
                         dict(e=dict(id=30, name="石碑", kind_label="物品", is_protagonist=False, importance=2, status_label="活跃", aliases=[],
                                     summary="", state="", state_after_anchor=False, attributes=[], description=""), relations=[], changes=[],
                              appearances=dict(first_label="", last_label="", window_size=10, window_count=0), design=None)],
  "result.entity_list": [dict(items=[dict(id=12, name="林凡", kind_label="人物", aliases=["凡哥"], summary="外门弟子", status="active", status_label="活跃"),
                                     dict(id=13, name="老剑", kind_label="物品", aliases=[], summary="", status="destroyed", status_label="已损毁")], total=40, shown=30),
                         dict(items=[], total=0, shown=0)],
  "result.relations": [dict(entity="林凡(#12)", items=[dict(subject="林凡(#12)", object="张三(#15)", kind="师兄", ended=False, until_label="", description="", since_label="")]),
                       dict(entity="林凡(#12)", items=[])],
  "result.timeline": [dict(items=[dict(id=1, story_time="春", title="入门", description="", chapter_label="第1章", entities=["林凡", "苏婉"], after_anchor=False)], total=5, shown=1, mode="chronological"),
                      dict(items=[], total=0, shown=0, mode="recent_first")],
  "result.foreshadowing_list": [dict(items=[dict(id=9, title="剑穗", status_label="未回收", description="……", planted_label="第5章", planned_payoff="", resolution="", resolved_label="")]),
                                dict(items=[])],
  "result.search_hits": [dict(corpus=c, query="剑冢", hits=[dict(kind_label="实体", title="剑冢", ref=3, snippet="……", location="范围", pids=["p3", "p4", "p5"] if c == "chapters" else [], after_anchor=False),
                                                             dict(kind_label="实体", title="剑冢", ref=4, snippet="……", location="范围", pids=["p9"] if c == "chapters" else [], after_anchor=True)], vector_available=v)
                         for c, v in [("canon", True), ("knowledge", False), ("chapters", True)]] + [dict(corpus="canon", query="x", hits=[], vector_available=True)],
  "result.knowledge_entry": [dict(id=7, title="宋代茶馆", scope="宋代社会", sources=["https://a"], content="……", offset=0, end=6000, total=9000, next_offset=6000),
                             dict(id=7, title="宋代茶馆", scope="宋代社会", sources=[], content="……", offset=0, end=100, total=100, next_offset=None)],
  "result.chapter_text": [dict(label="第3章《初见》", paragraphs=draft["paragraphs"], next_pid="p40", total_paragraphs=80),
                          dict(label="第3章", paragraphs=draft["paragraphs"], next_pid=None, total_paragraphs=2)],
  "result.draft_text": [dict(paragraphs=draft["paragraphs"], tail_open=True, word_count=12, target_min=3000, target_max=4000, next_pid=None, total_paragraphs=2),
                        dict(paragraphs=[], tail_open=False, word_count=0, target_min=3000, target_max=4000, next_pid=None, total_paragraphs=0)],
  "result.draft_edit": [dict(action=a, inserted=ins, appended_to=app, pid="p3", deleted=dele, after=aft, tail_open=to, last_pid="p9",
                             word_count=100, target_min=3000, target_max=4000, chapter_end=ce, short_of_target=sh, ignored_chapter_end=ice)
                        for a, ins, app, dele, aft, to, ce, sh, ice in [
                            ("insert_text", ["p9"], None, [], None, False, False, False, False),
                            ("insert_text", [], "p8", [], None, True, False, False, False),
                            ("insert_text", ["p9", "p10"], "p8", [], None, False, True, False, False),
                            ("insert_text", ["p9"], None, [], None, False, False, True, False),
                            ("insert_paragraphs", ["p11"], None, [], "p10", False, False, False, False),
                            ("insert_paragraphs", ["p1"], None, [], None, False, False, False, False),
                            ("replace_text", [], None, ["p3"], None, False, False, False, False),
                            ("rewrite_paragraph", ["p12"], None, [], None, False, False, False, True),
                            ("delete_paragraphs", [], None, ["p4", "p5"], None, False, False, False, False)]],
  "result.writer_control": [dict(action=a, word_count=1200 if c else 3100, target_min=3000, confirm_needed=c, title_set=t, title="剑冢", staged=st)
                            for a, c, t, st in [("submit_outline", False, True, False), ("submit_outline", False, False, False),
                                            ("submit_outline", False, True, True),
                                            ("submit_revision_plan", False, False, False),
                                            ("finish_draft", True, False, False), ("finish_draft", False, False, False), ("finish_revision", False, False, False)]],
  "result.submit_review": [dict(attempt=1, issues=3, dropped_pids=["p99"]), dict(attempt=2, issues=0, dropped_pids=[])],
  "result.canon_write": [dict(action=a, target=t, count=3, prev_event=p, next_event=n, moved=m) for a, t, p, n, m in [
                            ("create_entity", "苏婉(#21)", None, None, False), ("record_appearances", "", None, None, False),
                            ("add_timeline_event", "入门(#5)", 4, 6, False), ("add_timeline_event", "入门(#5)", None, None, False),
                            ("update_timeline_event", "入门(#5)", 2, None, True), ("set_chapter_summary", "", None, None, False),
                            ("propose_setting_change", "世界观", None, None, False),
                            ("propose_goal_change", "目标", None, None, False)]],
  "result.finish_maintenance": [dict(confirm_needed=True, changes=0, proposals=0, goal_proposals=0), dict(confirm_needed=False, changes=12, proposals=1, goal_proposals=2), dict(confirm_needed=False, changes=3, proposals=0, goal_proposals=0)],
  "result.ask_user": [dict(answers=[dict(question="基调？", selected=["热血"], other="再轻松点"), dict(question="结局？", selected=[], other=""),
                                    dict(question="篇幅？", selected=[], other="百万字")])],
  "result.request_review": [dict(items=[dict(doc_label="故事大纲", approved=True, feedback=""), dict(doc_label="世界观", approved=False, feedback="势力太多")],
                                 general="整体不错", all_approved=False),
                            dict(items=[dict(doc_label="故事大纲", approved=True, feedback="")], general="", all_approved=True)],
  "result.conception_write": [dict(action=a, doc_label="世界观", revision=3, chars=2000, name="林凡", rank=1, order=["林凡", "苏婉"], count=4, created=c)
                              for a, c in [("write_setting_doc", False), ("edit_setting_doc", False), ("upsert_character", True), ("upsert_character", False),
                                           ("remove_character", False), ("reorder_characters", False), ("set_research_topics", False)]],
  "result.web_search": [dict(query="宋代", hits=[dict(title="宋代茶馆", url="https://a", published="2020-01-01", snippet="……"), dict(title="b", url="https://b", published="", snippet="……")]),
                        dict(query="x", hits=[])],
  "result.web_fetch": [dict(url="https://a", title="标题", offset=0, end=8000, total=20000, next_offset=8000, body="正文"),
                       dict(url="https://a", title="", offset=0, end=100, total=100, next_offset=None, body="正文")],
  "result.knowledge_write": [dict(action="created", id=7, title="宋代茶馆", scope="宋代社会"), dict(action="updated", id=7, title="宋代茶馆", scope="宋代社会")],
  "result.finish_research": [dict(entries=5)],
}
error_cases = [
  dict(code="invalid_json", raw="{\"text\": \"…"), dict(code="validation", errors=["$.text: missing", "$.x: unexpected"]),
  dict(code="entity_not_found", query="林帆", suggestions=[dict(id=12, name="林凡")]), dict(code="entity_not_found", query="某人", suggestions=[]),
  dict(code="entity_ambiguous", query="小李", candidates=[dict(id=1, name="小李", kind_label="人物"), dict(id=2, name="小李子", kind_label="人物")]),
  dict(code="entity_exists", name="苏婉", id=21), dict(code="pid_not_found", tool="delete_paragraphs", pids=["p99", "p100"]), dict(code="pid_not_found", tool="read_chapter", pids=["p99"]), dict(code="text_not_found", pid="p3", text="原文"),
  dict(code="text_ambiguous", pid="p3", count=2),
  dict(code="granularity", tool="insert_text", mode="sentence", mode_label="逐句", reason="newline", length=0, limit=400),
  dict(code="granularity", tool="insert_paragraphs", mode="sentence", mode_label="逐句", reason="newline", length=0, limit=3000),
  dict(code="granularity", tool="insert_text", mode="paragraph", mode_label="逐段", reason="too_long", length=3500, limit=3000),
  dict(code="granularity", tool="insert_text", mode="free", mode_label="自由", reason="empty", length=0, limit=0),
  dict(code="draft_empty"), dict(code="isolated_readonly", target="林凡(#12)"), dict(code="chapter_not_found", query="99"),
  dict(code="chapter_not_official", label="第14章"), dict(code="no_knowledge_scope"),
  dict(code="scope_not_found", name="明代史料", available=["仙侠设定", "杂项"]), dict(code="not_found", what_label="时间线事件", id=3),
  dict(code="entry_exists", title="宋代茶馆", id=7), dict(code="find_failed", doc_label="世界观", count=0, find="三大宗门"),
  dict(code="find_failed", doc_label="世界观", count=2, find="宗门"), dict(code="find_failed", doc_label="知识条目", count=0, find="约三百家"), dict(code="maintenance_incomplete", missing=["summary", "appearances"]),
  dict(code="search_not_configured"), dict(code="search_failed", reason="timeout", status=0),
  dict(code="search_failed", reason="status", status=503), dict(code="search_failed", reason="auth", status=0), dict(code="fetch_failed", url="https://a", reason="timeout", status=0),
  dict(code="fetch_failed", url="https://a", reason="status", status=404), dict(code="fetch_failed", url="http://10.0.0.1", reason="blocked", status=0),
  dict(code="unsupported_content_type", url="https://a.pdf", content_type="application/pdf"), dict(code="character_not_found", name="王二"),
  dict(code="character_exists", name="苏婉"),
  dict(code="reorder_mismatch", expected=["林凡", "苏婉"]), dict(code="relation_not_found", source="林凡", target="张三", kind="师兄"),
  dict(code="seed_only", tool="create_entity", field="is_protagonist"), dict(code="exactly_one", fields=["edits", "append", "character"]),
  dict(code="mutually_exclusive", fields=["content", "edits"]), dict(code="mutually_exclusive", fields=["move_after", "move_before"]),
  dict(code="tool_not_allowed", tool="get_entity", phase_label="正文阶段", allowed=["insert_text"]), dict(code="output_truncated"),
  dict(code="after_completion"), dict(code="refusal"), dict(code="internal"),
]
variants["result.error"] = error_cases

rendered_count = 0
for key, entry in meta.items():
    declared = set(entry["vars"])
    for var in declared:
        if var not in variables["vars"] and not key.startswith(("notice.", "result.", "shared.", "writer.notice", "writer.rules")):
            errors.append(f"{key}: variable {var} not documented in variables.toml")
    cases = variants.get(key, [dict()])
    for i, override in enumerate(cases):
        ctx = {v: S[v] for v in declared if v in S}
        ctx.update(override)
        if key == "result.error":
            ctx = dict(override)
        else:
            unknown = set(ctx) - declared
            if unknown:
                errors.append(f"{key} case {i}: sample provides undeclared vars {unknown}")
        try:
            out = env.get_template(key).render(**ctx)
        except Exception as ex:
            errors.append(f"{key} case {i}: render error: {type(ex).__name__}: {ex}")
            continue
        rendered_count += 1
        out_norm = re.sub(r"\n{3,}", "\n\n", out.strip())
        if re.search(r"\bNone\b|\bnone\b", out_norm):
            errors.append(f"{key} case {i}: rendered output contains None:\n{out_norm[:400]}")
        if not out_norm and key not in {"block.goals", "block.canon_systems"}:
            errors.append(f"{key} case {i}: empty output")
        if "{{" in out_norm or "{%" in out_norm:
            errors.append(f"{key} case {i}: unrendered template syntax")
        if key in {"archivist.kickoff.maintenance", "archivist.kickoff.seed"}:
            numbers = [int(m.group(1)) for m in re.finditer(r"(?m)^(\d+)\.", out_norm)]
            if numbers != list(range(1, len(numbers) + 1)):
                errors.append(f"{key} case {i}: numbered steps are not contiguous from 1: {numbers}")
        glued = re.search(r"</?(book_outline|world_setting|protagonist_profile|character|character_profile|chapter_outline|reference_notes|chapter_text|writing_skill|document)\b[^>]*>[^\n]", out_norm)
        if glued:
            errors.append(f"{key} case {i}: tag line glued to following text: {glued.group(0)!r}")
        if len(sys.argv) > 2 and (sys.argv[2] == "all" or sys.argv[2] == key):
            print(f"===== {key} [{i}] =====\n{out_norm}\n")

# tool descriptions render as templates too
for name in tools:
    try:
        env.get_template(f"tool.{name}").render()
    except Exception as ex:
        errors.append(f"tool.{name}: {ex}")

print(f"templates: {len(meta)} prompt entries, {len(tools)} tools, {rendered_count} renders")
if errors:
    print(f"{len(errors)} problem(s):")
    for e in errors:
        print(" -", e)
    sys.exit(1)
print("OK")
