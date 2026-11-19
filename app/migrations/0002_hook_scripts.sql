-- The agent's startup and teardown scripts: bash, run as the agent's user around each start
-- of the agent (see Launch in command_plane.proto). Empty: none.
ALTER TABLE instance_revisions ADD COLUMN startup_script TEXT NOT NULL DEFAULT '';
ALTER TABLE instance_revisions ADD COLUMN teardown_script TEXT NOT NULL DEFAULT '';
