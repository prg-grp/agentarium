FROM agent-toolbox/base

# Claude Code, installed for the unprivileged ubuntu user (~/.local/bin/claude).
USER ubuntu
WORKDIR /home/ubuntu
RUN curl -fsSL https://claude.ai/install.sh | bash

# Claude starts in ~/workspace, already trusted. It needs its own folder, because Claude
# Code never saves trust for the home folder. The folder becomes configurable later.
RUN mkdir workspace && \
    jq '.projects["/home/ubuntu/workspace"].hasTrustDialogAccepted = true' .claude.json > .claude.json.new && \
    mv .claude.json.new .claude.json && chmod 600 .claude.json

# Agent mode: the console logs in ubuntu (see base), whose login runs Claude Code in
# ~/workspace. When it exits, the getty logs in again and starts it anew. Maintenance
# mode logs in root instead, so this never runs there (also not for `su - ubuntu`).
RUN printf '\n# Agent mode: run Claude Code on login.\ngrep -qw agent_toolbox.mode=maintenance /proc/cmdline || { cd "$HOME/workspace" && exec claude; }\n' >> .profile
