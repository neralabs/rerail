-- The schema at the migration baseline (schema v3), as the generated DDL created it. Never edit: a file
-- created from it and migrated by every step must match a new file (test/engine/storage/database-baseline.test.ts).

CREATE TABLE IF NOT EXISTS `sessions` (
	`session_id` text PRIMARY KEY NOT NULL,
	`host` text NOT NULL,
	`cwd` text NOT NULL,
	`repository_id` text,
	`started_at` integer NOT NULL,
	`ended_at` integer
);

CREATE INDEX IF NOT EXISTS `sessions_repository_started` ON `sessions` (`repository_id`,`started_at`);

CREATE TABLE IF NOT EXISTS `events` (
	`sequence` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`session_id` text NOT NULL,
	`event_id` text NOT NULL,
	`kind` text NOT NULL,
	`observed_at` integer NOT NULL,
	`payload_json` text NOT NULL,
	`payload_v8` blob,
	FOREIGN KEY (`session_id`) REFERENCES `sessions`(`session_id`) ON UPDATE no action ON DELETE no action
);

CREATE INDEX IF NOT EXISTS `events_session_sequence` ON `events` (`session_id`,`sequence`);

CREATE INDEX IF NOT EXISTS `events_session_kind_sequence` ON `events` (`session_id`,`kind`,`sequence`);

CREATE UNIQUE INDEX IF NOT EXISTS `events_session_event` ON `events` (`session_id`,`event_id`);

CREATE TABLE IF NOT EXISTS `rules` (
	`id` text PRIMARY KEY NOT NULL,
	`kind` text NOT NULL,
	`mode` text NOT NULL,
	`revision` integer NOT NULL,
	`enabled` integer DEFAULT true NOT NULL,
	`scope` text DEFAULT 'repository' NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	`source_digest` text,
	`format` text,
	`description` text,
	`scope_json` text,
	`clauses_json` text,
	`reference` text,
	`match_json` text,
	`sample_json` text,
	`action_json` text,
	`repeat_json` text,
	CONSTRAINT "rules_kind" CHECK("rules"."kind" IN ('code', 'conversation')),
	CONSTRAINT "rules_mode" CHECK("rules"."mode" IN ('log', 'steer', 'block')),
	CONSTRAINT "rules_revision" CHECK("rules"."revision" > 0),
	CONSTRAINT "rules_enabled" CHECK("rules"."enabled" IN (0, 1)),
	CONSTRAINT "rules_scope" CHECK("rules"."scope" IN ('repository', 'global')),
	CONSTRAINT "rules_format" CHECK("rules"."format" IS NULL OR "rules"."format" = 'rerail.policy/v1'),
	CONSTRAINT "rules_scope_json" CHECK("rules"."scope_json" IS NULL OR json_type("rules"."scope_json") = 'object'),
	CONSTRAINT "rules_clauses_json" CHECK("rules"."clauses_json" IS NULL OR json_type("rules"."clauses_json") = 'array'),
	CONSTRAINT "rules_match_json" CHECK("rules"."match_json" IS NULL OR json_type("rules"."match_json") = 'object'),
	CONSTRAINT "rules_sample_json" CHECK("rules"."sample_json" IS NULL OR json_type("rules"."sample_json") = 'object'),
	CONSTRAINT "rules_action_json" CHECK("rules"."action_json" IS NULL OR json_type("rules"."action_json") = 'object'),
	CONSTRAINT "rules_repeat_json" CHECK("rules"."repeat_json" IS NULL OR json_type("rules"."repeat_json") = 'object'),
	CONSTRAINT "rules_code_columns" CHECK("rules"."kind" <> 'code' OR (
        "rules"."format" IS NOT NULL
        AND "rules"."clauses_json" IS NOT NULL
        AND "rules"."reference" IS NULL
        AND "rules"."match_json" IS NULL
        AND "rules"."sample_json" IS NULL
        AND "rules"."action_json" IS NULL
        AND "rules"."repeat_json" IS NULL
      )),
	CONSTRAINT "rules_conversation_columns" CHECK("rules"."kind" <> 'conversation' OR (
        ("rules"."reference" IS NOT NULL OR json_extract("rules"."match_json", '$.operator') IS NULL)
        AND "rules"."match_json" IS NOT NULL
        AND "rules"."sample_json" IS NOT NULL
        AND "rules"."action_json" IS NOT NULL
        AND "rules"."format" IS NULL
        AND "rules"."description" IS NULL
        AND "rules"."scope_json" IS NULL
        AND "rules"."clauses_json" IS NULL
      ))
);

CREATE INDEX IF NOT EXISTS `rules_kind_enabled` ON `rules` (`kind`,`enabled`);

CREATE TABLE IF NOT EXISTS `work_links` (
	`link_id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`session_id` text NOT NULL,
	`kind` text NOT NULL,
	`reference` text NOT NULL,
	`url` text,
	`created_at` integer NOT NULL
);

CREATE INDEX IF NOT EXISTS `work_links_reference` ON `work_links` (`kind`,`reference`);

CREATE UNIQUE INDEX IF NOT EXISTS `work_links_session_reference` ON `work_links` (`session_id`,`kind`,`reference`);

CREATE TABLE IF NOT EXISTS `audits` (
	`audit_id` text PRIMARY KEY NOT NULL,
	`session_id` text NOT NULL,
	`workspace_root` text NOT NULL,
	`scope` text NOT NULL,
	`mode` text NOT NULL,
	`revision` integer NOT NULL,
	`catalog_revision` text,
	`rule_revisions_json` text DEFAULT '[]' NOT NULL,
	`rule_provenance_json` text NOT NULL,
	`paths_json` text NOT NULL,
	`started_at` integer NOT NULL,
	`finished_at` integer,
	`status` text NOT NULL,
	`violations_json` text NOT NULL,
	`failures_json` text NOT NULL,
	CONSTRAINT "audits_scope" CHECK("audits"."scope" IN ('repository', 'touched-files', 'ai-output')),
	CONSTRAINT "audits_mode" CHECK("audits"."mode" IN ('steer', 'log')),
	CONSTRAINT "audits_status" CHECK("audits"."status" IN ('running', 'completed', 'failed', 'interrupted', 'superseded')),
	CONSTRAINT "audits_revision" CHECK("audits"."revision" >= 0),
	CONSTRAINT "audits_rule_provenance_json" CHECK(json_type("audits"."rule_provenance_json") = 'array'),
	CONSTRAINT "audits_paths_json" CHECK(json_type("audits"."paths_json") = 'array'),
	CONSTRAINT "audits_violations_json" CHECK(json_type("audits"."violations_json") = 'array'),
	CONSTRAINT "audits_failures_json" CHECK(json_type("audits"."failures_json") = 'array')
);

CREATE INDEX IF NOT EXISTS `audits_session_started` ON `audits` (`session_id`,`started_at`,`audit_id`);

CREATE INDEX IF NOT EXISTS `audits_status` ON `audits` (`status`);

CREATE INDEX IF NOT EXISTS `audits_scope` ON `audits` (`scope`);

CREATE INDEX IF NOT EXISTS `audits_mode` ON `audits` (`mode`);

CREATE TABLE IF NOT EXISTS `audit_result_pages` (
	`audit_id` text NOT NULL,
	`page_index` integer NOT NULL,
	`violations_json` text NOT NULL,
	`failures_json` text NOT NULL,
	PRIMARY KEY(`audit_id`, `page_index`),
	FOREIGN KEY (`audit_id`) REFERENCES `audits`(`audit_id`) ON UPDATE no action ON DELETE cascade,
	CONSTRAINT "audit_result_pages_index" CHECK("audit_result_pages"."page_index" >= 0),
	CONSTRAINT "audit_result_pages_violations_json" CHECK(json_type("audit_result_pages"."violations_json") = 'array'),
	CONSTRAINT "audit_result_pages_failures_json" CHECK(json_type("audit_result_pages"."failures_json") = 'array')
);

CREATE TABLE IF NOT EXISTS `presentation_preferences` (
	`repository` text NOT NULL,
	`host` text NOT NULL,
	`presence` text NOT NULL,
	PRIMARY KEY(`repository`, `host`),
	CONSTRAINT "presentation_preferences_host" CHECK("presentation_preferences"."host" IN ('', 'omp', 'pi', 'claude-code', 'codex', 'cursor')),
	CONSTRAINT "presentation_preferences_presence" CHECK("presentation_preferences"."presence" IN ('compact', 'silent'))
);

CREATE TABLE IF NOT EXISTS `audit_settings` (
	`workspace_root` text PRIMARY KEY NOT NULL,
	`settings_json` text NOT NULL,
	`updated_at` integer NOT NULL
);

CREATE TABLE IF NOT EXISTS `repository_audits` (
	`workspace_root` text PRIMARY KEY NOT NULL,
	`session_id` text NOT NULL,
	`audit_id` text,
	`status` text NOT NULL,
	`started_at` integer NOT NULL,
	`finished_at` integer,
	`error` text,
	CONSTRAINT "repository_audits_status" CHECK("repository_audits"."status" IN ('running', 'completed', 'failed', 'interrupted'))
);

CREATE TABLE IF NOT EXISTS `audit_requests` (
	`id` text PRIMARY KEY NOT NULL,
	`session_id` text NOT NULL,
	`workspace_root` text NOT NULL,
	`scope` text NOT NULL,
	`mode` text NOT NULL,
	`paths_json` text NOT NULL,
	`status` text NOT NULL,
	`created_at` integer NOT NULL,
	`started_at` integer,
	`finished_at` integer,
	`audit_ids_json` text NOT NULL,
	`error` text,
	CONSTRAINT "audit_requests_scope" CHECK("audit_requests"."scope" IN ('repository', 'touched-files', 'ai-output')),
	CONSTRAINT "audit_requests_mode" CHECK("audit_requests"."mode" IN ('steer', 'log')),
	CONSTRAINT "audit_requests_status_value" CHECK("audit_requests"."status" IN ('queued', 'running', 'completed', 'failed', 'interrupted')),
	CONSTRAINT "audit_requests_paths_json" CHECK(json_type("audit_requests"."paths_json") = 'array'),
	CONSTRAINT "audit_requests_audit_ids_json" CHECK(json_type("audit_requests"."audit_ids_json") = 'array')
);

CREATE INDEX IF NOT EXISTS `audit_requests_session_created` ON `audit_requests` (`session_id`,`created_at`,`id`);

CREATE INDEX IF NOT EXISTS `audit_requests_status` ON `audit_requests` (`status`);

CREATE TABLE IF NOT EXISTS `policy_session_owners` (
	`session_id` text PRIMARY KEY NOT NULL,
	`delivery` text NOT NULL,
	`active` integer NOT NULL,
	`process_id` integer NOT NULL,
	`token` text NOT NULL,
	`updated_at` integer NOT NULL
);

CREATE TABLE IF NOT EXISTS `rule_catalog_identities` (
	`rule_id` text PRIMARY KEY NOT NULL,
	`kind` text NOT NULL,
	CONSTRAINT "rule_catalog_identities_kind" CHECK("rule_catalog_identities"."kind" IN ('code', 'conversation'))
);

CREATE TABLE IF NOT EXISTS `repositories` (
	`repository_key` text PRIMARY KEY NOT NULL,
	`display_name` text NOT NULL,
	`aliases_json` text DEFAULT '[]' NOT NULL,
	`configured_at` integer,
	`started` integer,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT "repositories_aliases" CHECK(json_type("repositories"."aliases_json") = 'array')
);

CREATE TABLE IF NOT EXISTS `checkouts` (
	`checkout_path` text PRIMARY KEY NOT NULL,
	`repository_key` text NOT NULL,
	`clone_id` text NOT NULL,
	`started` integer,
	`created_at` integer NOT NULL,
	`last_seen_at` integer NOT NULL,
	FOREIGN KEY (`repository_key`) REFERENCES `repositories`(`repository_key`) ON UPDATE cascade ON DELETE cascade
);

CREATE INDEX IF NOT EXISTS `checkouts_clone` ON `checkouts` (`clone_id`);

CREATE INDEX IF NOT EXISTS `checkouts_repository` ON `checkouts` (`repository_key`);

CREATE TABLE IF NOT EXISTS `setup_reviews` (
	`repository_key` text PRIMARY KEY NOT NULL,
	`offered_at` integer NOT NULL,
	FOREIGN KEY (`repository_key`) REFERENCES `repositories`(`repository_key`) ON UPDATE cascade ON DELETE cascade
);

CREATE TABLE IF NOT EXISTS `rule_attachments` (
	`repository_key` text NOT NULL,
	`folder` text DEFAULT '' NOT NULL,
	`source_kind` text NOT NULL,
	`source_id` text NOT NULL,
	`created_at` integer NOT NULL,
	PRIMARY KEY(`repository_key`, `folder`, `source_kind`, `source_id`),
	FOREIGN KEY (`repository_key`) REFERENCES `repositories`(`repository_key`) ON UPDATE cascade ON DELETE cascade,
	CONSTRAINT "rule_attachments_source_kind" CHECK("rule_attachments"."source_kind" IN ('pack', 'rerail_rule', 'library_rule'))
);

CREATE TABLE IF NOT EXISTS `rule_settings` (
	`owner_kind` text NOT NULL,
	`owner_id` text NOT NULL,
	`folder` text DEFAULT '' NOT NULL,
	`target_kind` text NOT NULL,
	`target_id` text NOT NULL,
	`on` integer,
	`mode` text,
	`updated_at` integer NOT NULL,
	PRIMARY KEY(`owner_kind`, `owner_id`, `folder`, `target_kind`, `target_id`),
	CONSTRAINT "rule_settings_owner_kind" CHECK("rule_settings"."owner_kind" IN ('repository', 'checkout')),
	CONSTRAINT "rule_settings_target_kind" CHECK("rule_settings"."target_kind" IN ('rule', 'pack')),
	CONSTRAINT "rule_settings_mode" CHECK("rule_settings"."mode" IS NULL OR "rule_settings"."mode" IN ('log', 'steer', 'block')),
	CONSTRAINT "rule_settings_value" CHECK("rule_settings"."on" IS NOT NULL OR "rule_settings"."mode" IS NOT NULL)
);

CREATE TABLE IF NOT EXISTS `library_rules` (
	`rule_id` text PRIMARY KEY NOT NULL,
	`kind` text NOT NULL,
	`document_json` text NOT NULL,
	`revision` integer NOT NULL,
	`payload_hash` text NOT NULL,
	`copied_from_rule_id` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	CONSTRAINT "library_rules_kind" CHECK("library_rules"."kind" IN ('code', 'conversation')),
	CONSTRAINT "library_rules_revision" CHECK("library_rules"."revision" > 0)
);

CREATE TABLE IF NOT EXISTS `schema_migrations` (
	`id` text PRIMARY KEY NOT NULL,
	`applied_at` integer NOT NULL
);
