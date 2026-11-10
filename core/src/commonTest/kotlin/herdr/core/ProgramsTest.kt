package herdr.core

import kotlin.test.Test
import kotlin.test.assertEquals

// The desktop's own cases (herdr-app src/programs.rs).
class ProgramsTest {
    private fun key(app: String?, vararg argv: String) = programOf(app, argv.toList()).key

    @Test
    fun agentsShellsToolsAndWhatInterpretersRun() {
        assertEquals("claude", key("claude-code", "/bin/zsh"))
        assertEquals("agent", key("newagent"))
        assertEquals("shell", key(null, "/bin/zsh"))
        assertEquals("shell", key(null, "-zsh"))
        assertEquals("nvim", key(null, "nvim", "x.rs"))
        assertEquals("vim", key(null, "vi"))
        assertEquals("vite", key(null, "node", "/x/node_modules/.bin/vite", "dev"))
        assertEquals("vitest", key(null, "npx", "vitest"))
        assertEquals("node", key(null, "node", "server.js"))
        assertEquals("python", key(null, "python3", "-m", "http.server", "8799"))
        assertEquals("pytest", key(null, "uv", "run", "pytest"))
        assertEquals("program", key(null, "mytool"))
        assertEquals("vim", key(null, "/bin/zsh", "-ic", "\"\$@\"; exec \"\$0\" -i", "/bin/zsh", "vi", "x"))
        assertEquals("vim", key(null, "/usr/bin/fish", "-ic", "command \$argv[2..]; exec \$argv[1] -i", "/usr/bin/fish", "vi", "x"))
        assertEquals("codex", key(null, "pwsh.exe", "-NoExit", "-Command", "& 'codex' 'fix it'"))
        assertEquals("shell", key(null, "C:\\Windows\\powershell.exe"))
    }

    @Test
    fun shownLikeTheDesktop() {
        val claude = programOf("claude-code", emptyList())
        assertEquals("Claude Code", claude.name)
        assertEquals("icons/claude.svg", claude.icon)
        assertEquals(false, claude.tint)
        val tool = programOf(null, listOf("mytool"))
        assertEquals("mytool", tool.name)
        assertEquals("#ffffff", tool.color)
        assertEquals(tool.tile, programOf(null, listOf("mytool")).tile)
        assertEquals("fish", programOf(null, listOf("/usr/bin/fish")).name)
    }
}
