// Types of what the nonio server sends the page, matching the Rust types' JSON. Generated once from nonio's
// packages/core and packages/server types (dts-bundle-generator); kept in step with the Rust structs since.
import type { z } from 'zod';


declare const Screen$1: z.ZodObject<{
	kind: z.ZodEnum<{
		image: "image";
		capture: "capture";
		sketch: "sketch";
		wireframe: "wireframe";
	}>;
	capture: z.ZodOptional<z.ZodString>;
	page: z.ZodOptional<z.ZodString>;
	region: z.ZodOptional<z.ZodString>;
	file: z.ZodOptional<z.ZodString>;
	elements: z.ZodOptional<z.ZodArray<z.ZodObject<{
		kind: z.ZodEnum<{
			text: "text";
			heading: "heading";
			button: "button";
			field: "field";
			link: "link";
			image: "image";
		}>;
		text: z.ZodString;
		proposed: z.ZodOptional<z.ZodBoolean>;
	}, z.core.$strip>>>;
	highlight: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
declare const Slot: z.ZodEnum<{
	top: "top";
	left: "left";
	main: "main";
	right: "right";
	bottom: "bottom";
}>;
declare const Proposal: z.ZodObject<{
	kind: z.ZodEnum<{
		add: "add";
		change: "change";
		remove: "remove";
	}>;
	note: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
declare const Node$1: z.ZodObject<{
	id: z.ZodString;
	area: z.ZodString;
	kind: z.ZodDefault<z.ZodEnum<{
		entry: "entry";
		step: "step";
		store: "store";
		external: "external";
		job: "job";
		screen: "screen";
		action: "action";
		choice: "choice";
	}>>;
	name: z.ZodString;
	symbol: z.ZodOptional<z.ZodString>;
	explain: z.ZodOptional<z.ZodObject<{
		concept: z.ZodOptional<z.ZodString>;
		how: z.ZodOptional<z.ZodString>;
		code: z.ZodOptional<z.ZodString>;
	}, z.core.$strip>>;
	evidence: z.ZodDefault<z.ZodArray<z.ZodObject<{
		file: z.ZodString;
		line: z.ZodNumber;
		text: z.ZodString;
	}, z.core.$strip>>>;
	screen: z.ZodOptional<z.ZodObject<{
		kind: z.ZodEnum<{
			image: "image";
			capture: "capture";
			sketch: "sketch";
			wireframe: "wireframe";
		}>;
		capture: z.ZodOptional<z.ZodString>;
		page: z.ZodOptional<z.ZodString>;
		region: z.ZodOptional<z.ZodString>;
		file: z.ZodOptional<z.ZodString>;
		elements: z.ZodOptional<z.ZodArray<z.ZodObject<{
			kind: z.ZodEnum<{
				text: "text";
				heading: "heading";
				button: "button";
				field: "field";
				link: "link";
				image: "image";
			}>;
			text: z.ZodString;
			proposed: z.ZodOptional<z.ZodBoolean>;
		}, z.core.$strip>>>;
		highlight: z.ZodOptional<z.ZodString>;
	}, z.core.$strip>>;
	proposed: z.ZodOptional<z.ZodObject<{
		kind: z.ZodEnum<{
			add: "add";
			change: "change";
			remove: "remove";
		}>;
		note: z.ZodOptional<z.ZodString>;
	}, z.core.$strip>>;
	removed: z.ZodOptional<z.ZodBoolean>;
	branch: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
declare const Edge: z.ZodObject<{
	id: z.ZodString;
	from: z.ZodString;
	to: z.ZodString;
	kind: z.ZodDefault<z.ZodEnum<{
		calls: "calls";
		request: "request";
		imports: "imports";
		then: "then";
	}>>;
	label: z.ZodString;
	evidence: z.ZodDefault<z.ZodArray<z.ZodObject<{
		file: z.ZodString;
		line: z.ZodNumber;
		text: z.ZodString;
	}, z.core.$strip>>>;
	proposed: z.ZodOptional<z.ZodObject<{
		kind: z.ZodEnum<{
			add: "add";
			change: "change";
			remove: "remove";
		}>;
		note: z.ZodOptional<z.ZodString>;
	}, z.core.$strip>>;
	removed: z.ZodOptional<z.ZodBoolean>;
	branch: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
declare const Absence: z.ZodObject<{
	id: z.ZodString;
	claim: z.ZodString;
	between: z.ZodOptional<z.ZodObject<{
		from: z.ZodString;
		to: z.ZodString;
	}, z.core.$strip>>;
	search: z.ZodObject<{
		pattern: z.ZodString;
		regex: z.ZodDefault<z.ZodBoolean>;
		paths: z.ZodOptional<z.ZodArray<z.ZodString>>;
	}, z.core.$strip>;
	matches: z.ZodNumber;
}, z.core.$strip>;
export declare const TourStop: z.ZodObject<{
	id: z.ZodString;
	title: z.ZodString;
	text: z.ZodString;
	focus: z.ZodObject<{
		areas: z.ZodOptional<z.ZodArray<z.ZodString>>;
		nodes: z.ZodOptional<z.ZodArray<z.ZodString>>;
		edges: z.ZodOptional<z.ZodArray<z.ZodString>>;
		absences: z.ZodOptional<z.ZodArray<z.ZodString>>;
	}, z.core.$strip>;
	terms: z.ZodOptional<z.ZodArray<z.ZodString>>;
	ask: z.ZodOptional<z.ZodArray<z.ZodString>>;
}, z.core.$strip>;
export declare const CodeMap: z.ZodObject<{
	version: z.ZodLiteral<1>;
	question: z.ZodString;
	commit: z.ZodOptional<z.ZodNullable<z.ZodString>>;
	view: z.ZodOptional<z.ZodEnum<{
		journey: "journey";
		timeline: "timeline";
	}>>;
	since: z.ZodOptional<z.ZodString>;
	pages: z.ZodOptional<z.ZodArray<z.ZodObject<{
		id: z.ZodString;
		name: z.ZodString;
		file: z.ZodString;
		regions: z.ZodArray<z.ZodObject<{
			id: z.ZodString;
			slot: z.ZodEnum<{
				top: "top";
				left: "left";
				main: "main";
				right: "right";
				bottom: "bottom";
			}>;
			label: z.ZodString;
			file: z.ZodString;
			elements: z.ZodDefault<z.ZodArray<z.ZodObject<{
				kind: z.ZodEnum<{
					text: "text";
					heading: "heading";
					button: "button";
					field: "field";
					link: "link";
					image: "image";
				}>;
				text: z.ZodString;
				proposed: z.ZodOptional<z.ZodBoolean>;
			}, z.core.$strip>>>;
		}, z.core.$strip>>;
	}, z.core.$strip>>>;
	areas: z.ZodArray<z.ZodObject<{
		id: z.ZodString;
		label: z.ZodString;
	}, z.core.$strip>>;
	nodes: z.ZodArray<z.ZodObject<{
		id: z.ZodString;
		area: z.ZodString;
		kind: z.ZodDefault<z.ZodEnum<{
			entry: "entry";
			step: "step";
			store: "store";
			external: "external";
			job: "job";
			screen: "screen";
			action: "action";
			choice: "choice";
		}>>;
		name: z.ZodString;
		symbol: z.ZodOptional<z.ZodString>;
		explain: z.ZodOptional<z.ZodObject<{
			concept: z.ZodOptional<z.ZodString>;
			how: z.ZodOptional<z.ZodString>;
			code: z.ZodOptional<z.ZodString>;
		}, z.core.$strip>>;
		evidence: z.ZodDefault<z.ZodArray<z.ZodObject<{
			file: z.ZodString;
			line: z.ZodNumber;
			text: z.ZodString;
		}, z.core.$strip>>>;
		screen: z.ZodOptional<z.ZodObject<{
			kind: z.ZodEnum<{
				image: "image";
				capture: "capture";
				sketch: "sketch";
				wireframe: "wireframe";
			}>;
			capture: z.ZodOptional<z.ZodString>;
			page: z.ZodOptional<z.ZodString>;
			region: z.ZodOptional<z.ZodString>;
			file: z.ZodOptional<z.ZodString>;
			elements: z.ZodOptional<z.ZodArray<z.ZodObject<{
				kind: z.ZodEnum<{
					text: "text";
					heading: "heading";
					button: "button";
					field: "field";
					link: "link";
					image: "image";
				}>;
				text: z.ZodString;
				proposed: z.ZodOptional<z.ZodBoolean>;
			}, z.core.$strip>>>;
			highlight: z.ZodOptional<z.ZodString>;
		}, z.core.$strip>>;
		proposed: z.ZodOptional<z.ZodObject<{
			kind: z.ZodEnum<{
				add: "add";
				change: "change";
				remove: "remove";
			}>;
			note: z.ZodOptional<z.ZodString>;
		}, z.core.$strip>>;
		removed: z.ZodOptional<z.ZodBoolean>;
		branch: z.ZodOptional<z.ZodString>;
	}, z.core.$strip>>;
	edges: z.ZodDefault<z.ZodArray<z.ZodObject<{
		id: z.ZodString;
		from: z.ZodString;
		to: z.ZodString;
		kind: z.ZodDefault<z.ZodEnum<{
			calls: "calls";
			request: "request";
			imports: "imports";
			then: "then";
		}>>;
		label: z.ZodString;
		evidence: z.ZodDefault<z.ZodArray<z.ZodObject<{
			file: z.ZodString;
			line: z.ZodNumber;
			text: z.ZodString;
		}, z.core.$strip>>>;
		proposed: z.ZodOptional<z.ZodObject<{
			kind: z.ZodEnum<{
				add: "add";
				change: "change";
				remove: "remove";
			}>;
			note: z.ZodOptional<z.ZodString>;
		}, z.core.$strip>>;
		removed: z.ZodOptional<z.ZodBoolean>;
		branch: z.ZodOptional<z.ZodString>;
	}, z.core.$strip>>>;
	absences: z.ZodDefault<z.ZodArray<z.ZodObject<{
		id: z.ZodString;
		claim: z.ZodString;
		between: z.ZodOptional<z.ZodObject<{
			from: z.ZodString;
			to: z.ZodString;
		}, z.core.$strip>>;
		search: z.ZodObject<{
			pattern: z.ZodString;
			regex: z.ZodDefault<z.ZodBoolean>;
			paths: z.ZodOptional<z.ZodArray<z.ZodString>>;
		}, z.core.$strip>;
		matches: z.ZodNumber;
	}, z.core.$strip>>>;
	tour: z.ZodDefault<z.ZodArray<z.ZodObject<{
		id: z.ZodString;
		title: z.ZodString;
		text: z.ZodString;
		focus: z.ZodObject<{
			areas: z.ZodOptional<z.ZodArray<z.ZodString>>;
			nodes: z.ZodOptional<z.ZodArray<z.ZodString>>;
			edges: z.ZodOptional<z.ZodArray<z.ZodString>>;
			absences: z.ZodOptional<z.ZodArray<z.ZodString>>;
		}, z.core.$strip>;
		terms: z.ZodOptional<z.ZodArray<z.ZodString>>;
		ask: z.ZodOptional<z.ZodArray<z.ZodString>>;
	}, z.core.$strip>>>;
}, z.core.$strip>;
type Screen$1 = z.output<typeof Screen$1>;
type Proposal = z.output<typeof Proposal>;
type Slot = z.output<typeof Slot>;
export type MapNode = z.output<typeof Node$1>;
export type MapEdge = z.output<typeof Edge>;
export type MapAbsence = z.output<typeof Absence>;
export type TourStop = z.output<typeof TourStop>;
export type CodeMap = z.output<typeof CodeMap>;
type ScreenBox = {
	x: number;
	y: number;
	w: number;
	h: number;
};
export type ScreenShow = {
	kind: "capture";
	capture: string;
	title: string;
	width: number;
	height: number;
	from: string;
	box: ScreenBox | null;
} | {
	kind: "image";
	file: string;
} | {
	kind: "sketch";
	file: string;
	elements: NonNullable<Screen$1["elements"]>;
	highlight: string | null;
} | {
	kind: "wireframe";
	page: PageShow;
	region: string | null;
	highlight: string | null;
};
type PageShow = {
	id: string;
	name: string;
	file: string;
	regions: {
		id: string;
		slot: Slot;
		label: string;
		file: string;
		elements: {
			kind: string;
			text: string;
		}[];
	}[];
};
type ScreenCheck = {
	status: "confirmed" | "unconfirmed";
	reasons: string[];
	show: ScreenShow | null;
};
type Status = "confirmed" | "unconfirmed" | "proposed";
type ProposalCheck = {
	kind: Proposal["kind"];
	state: "pending" | "done";
};
type Proof = "fact" | "text";
type EvidenceCheck = {
	file: string;
	line: number;
	ok: boolean;
	foundAt: number | null;
	reason: string | null;
};
type ChangeCheck = {
	kind: "added" | "changed" | "removed";
	diff?: string[];
};
export type ClaimCheck = {
	status: Status;
	proof: Proof | null;
	evidence: EvidenceCheck[];
	reasons: string[];
	proposal?: ProposalCheck;
	change?: ChangeCheck;
};
type SinceCheck = {
	ref: string;
	commit: string | null;
	date: string | null;
	subject: string | null;
};
/** `searched` is how many files the search covered, so a person can judge whether it looked widely enough. */
export type AbsenceCheck = {
	status: Exclude<Status, "proposed">;
	found: number;
	searched: number;
	locations: {
		file: string;
		line: number;
	}[];
	reasons: string[];
};
type TourCheck = {
	status: "ok" | "mentions-unconfirmed";
	unconfirmed: string[];
};
export type CheckReport = {
	nodes: Record<string, ClaimCheck>;
	edges: Record<string, ClaimCheck>;
	absences: Record<string, AbsenceCheck>;
	tour: Record<string, TourCheck>;
	/** By box id, for boxes that carry a screen. Not counted as claims: a screen that fails is simply not shown. */
	screens: Record<string, ScreenCheck>;
	/** `claims` counts what the map says about the code as it is; what is only proposed is counted under `proposals`. */
	summary: {
		claims: number;
		confirmed: number;
		unconfirmed: number;
		proposals: {
			pending: number;
			done: number;
		};
		changes?: {
			added: number;
			changed: number;
			removed: number;
		};
	};
	/** Only on a map of a change. */
	since?: SinceCheck;
};
export type CodeBlock = {
	file: string;
	/** The function, class or type these lines are in, as the code facts name it. Null when the facts know of none. */
	name: string | null;
	/** Whether the block is all of it, or only the lines around what is cited. */
	whole: boolean;
	/** The line number of the first line. */
	start: number;
	lines: string[];
	/** Line numbers the box, arrow or answer cites. */
	cited: number[];
	notes: {
		line: number;
		note: string;
	}[];
};
export type CodeView = {
	/** `before`: the code as it was at the commit a map of a change is compared with. */
	when: "now" | "before";
	blocks: CodeBlock[];
	/** Citations or notes whose line that version of the code doesn't have, so nothing is shown for them. */
	missing: number;
};
declare const RawFix: z.ZodObject<{
	summary: z.ZodString;
	edits: z.ZodDefault<z.ZodArray<z.ZodObject<{
		file: z.ZodString;
		find: z.ZodString;
		replace: z.ZodString;
	}, z.core.$strip>>>;
	creates: z.ZodDefault<z.ZodArray<z.ZodObject<{
		file: z.ZodString;
		content: z.ZodString;
	}, z.core.$strip>>>;
	connects: z.ZodOptional<z.ZodObject<{
		from: z.ZodString;
		to: z.ZodString;
		label: z.ZodString;
		evidenceFile: z.ZodString;
		evidenceText: z.ZodString;
	}, z.core.$strip>>;
}, z.core.$strip>;
type RawFix = z.output<typeof RawFix>;
type FixFile = {
	file: string;
	change: "edit" | "create";
	hash: string | null;
	uncommitted: boolean;
};
export type FixProposal = {
	id: string;
	summary: string;
	edits: RawFix["edits"];
	creates: RawFix["creates"];
	connects: NonNullable<RawFix["connects"]> | null;
	files: FixFile[];
	/** A readable before/after of every change, for the person approving it. */
	diff: string;
};
type Selection$1 = {
	nodes?: string[];
	edges?: string[];
	absences?: string[];
};
type RefStatus = {
	id: string;
	status: Status | "unknown";
};
export type Answer = {
	id: string;
	question: string;
	selection: Selection$1;
	/** The answer, in a few plain sentences. (Answers saved before replies were trimmed may also carry `how` and `code`.) */
	concept: string;
	refs: RefStatus[];
	moves: {
		focus: string | null;
		highlight: string[];
	};
	/** Ids drawn on the map because of this answer (all confirmed, or proposals the checker placed). */
	added: string[];
	/** Boxes and arrows already on the map that this answer marked as to change or remove. Missing on answers saved before proposals existed. */
	proposed?: string[];
	/** Boxes whose picture of the screen this answer set (all confirmed). Missing on answers saved before screens existed. */
	screens?: string[];
	/**
	 * Lines of code the answer points at, each with what it says about that line. Only ones the checker found are kept,
	 * and they are checked again whenever they are shown. Missing on answers saved before notes existed.
	 */
	notes?: CodeNote[];
	/** What was found but not drawn, and why; problems with a proposed fix. */
	problems: string[];
	fix: FixProposal | null;
	/** `map`: answered from the map alone. `code`: Claude looked in the code. */
	source: "map" | "code";
	/**
	 * Set when the question is about a different part of the app than this map covers: there's no answer yet, only the
	 * offer of a new map (or an existing one), or an answer in words. Missing on answers saved before offers existed.
	 */
	offer?: Offer | null;
	cost: number;
};
type Offer = {
	/** The question as a new map would ask it. */
	question: string;
	/** A saved map that already covers it. */
	existing: {
		id: string;
		question: string;
	} | null;
	status: "pending" | "mapped" | "answered";
	/** The map made for it, once they said yes. */
	mapId: string | null;
};
export type MapMeta = {
	id: string;
	question: string;
	createdAt: string;
	updatedAt: string;
	commit: string | null;
	makerSession: string | null;
	explainerSession: string | null;
	cost: number;
	/** The map this one was started from, when a question there turned out to be about something else. */
	parent?: {
		mapId: string;
		question: string;
	} | null;
};
type JobKind = "make" | "ask";
/**
 * Who does the model work, as the page shows it: `name` in "… is working", and where questions are asked. `page`: in
 * the map's own question box. `chat`: in the agent's chat beside the page (Codex), which then asks nonio.
 */
export type PageAgent = {
	name: string;
	asks: "page" | "chat";
};
export type ServerEvent = {
	type: "job";
	mapId: string;
	job: JobKind;
	status: "started" | "done" | "failed";
	question: string;
	error?: string;
} | {
	type: "progress";
	mapId: string;
	job: JobKind;
	kind: string;
	text: string;
} | {
	type: "map";
	mapId: string;
	meta: MapMeta;
	map: CodeMap;
	report: CheckReport;
} | {
	type: "answer";
	mapId: string;
	answer: Answer;
}
/** An answer already sent changed: its offer of a new map was taken up or turned down. */
 | {
	type: "answerUpdate";
	mapId: string;
	answer: Answer;
} | {
	type: "fix";
	mapId: string;
	fixId: string;
	status: "applied" | "rejected" | "undone" | "failed";
	problems?: string[];
	edgeId?: string | null;
	closed?: string[];
};

export {
	Selection$1 as Selection,
};

export {};
