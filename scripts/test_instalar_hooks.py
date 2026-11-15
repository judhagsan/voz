import copy
import json
import shlex
import tempfile
import unittest
from pathlib import Path

from instalar_hooks import (
    BLOCO_INSTRUCAO,
    aplicar_bloco_instrucao,
    carregar,
    configurar_antigravity,
    configurar_claude,
    configurar_codex,
    configurar_instrucao,
    salvar,
)


class InstalacaoHooksTest(unittest.TestCase):
    def test_codex_preserva_ai_memory_e_nao_duplica_hooks(self):
        memoria = {"matcher": "", "hooks": [{"type": "command", "command": "ai-memory hook --event stop"}]}
        dados = {"description": "Meus hooks", "hooks": {"Stop": [copy.deepcopy(memoria)], "PreToolUse": [copy.deepcopy(memoria)]}}
        configurar_codex(dados, Path("/pasta com espaços/voz.sh"))
        primeira = copy.deepcopy(dados)
        configurar_codex(dados, Path("/pasta com espaços/voz.sh"))
        self.assertEqual(dados, primeira)
        self.assertEqual(dados["hooks"]["Stop"][0], memoria)
        self.assertEqual(dados["hooks"]["PreToolUse"][0], memoria)
        self.assertEqual(dados["description"], "Meus hooks")
        self.assertEqual(len(dados["hooks"]["Stop"]), 2)
        for evento in ("Stop", "PermissionRequest", "PreToolUse"):
            hook = dados["hooks"][evento][-1]["hooks"][0]
            self.assertEqual(shlex.split(hook["command"]), ["/pasta com espaços/voz.sh", "--codex"])
            self.assertTrue(hook["async"])
        # Ao mudar o caminho do repositório, atualiza o comando existente sem criar duplicatas
        configurar_codex(dados, Path("/novo/caminho/voz.sh"))
        self.assertEqual(len(dados["hooks"]["Stop"]), 2)
        hook_atualizado = dados["hooks"]["Stop"][-1]["hooks"][0]
        self.assertEqual(shlex.split(hook_atualizado["command"]), ["/novo/caminho/voz.sh", "--codex"])

    def test_salvar_faz_backup_uma_vez_e_instalacao_repetida_nao_reescreve(self):
        with tempfile.TemporaryDirectory() as pasta:
            arquivo = Path(pasta) / "hooks.json"
            original = '{"hooks": {"Stop": []}}\n'
            arquivo.write_text(original)
            dados = carregar(arquivo)
            configurar_codex(dados, Path("/voz/voz.sh"))
            salvar(arquivo, dados)
            mtime = arquivo.stat().st_mtime_ns
            salvar(arquivo, dados)
            self.assertEqual(arquivo.stat().st_mtime_ns, mtime)
            self.assertEqual(arquivo.with_name("hooks.json.voz.bak").read_text(), original)
            self.assertEqual(carregar(arquivo), dados)

    def test_json_invalido_nao_e_descartado(self):
        with tempfile.TemporaryDirectory() as pasta:
            arquivo = Path(pasta) / "hooks.json"
            arquivo.write_text("{json incompleto")
            with self.assertRaises(ValueError):
                carregar(arquivo)
            self.assertEqual(arquivo.read_text(), "{json incompleto")

    def test_preserva_integracoes_claude_e_antigravity(self):
        claude = {"hooks": {"Stop": [{"hooks": [{"command": "ai-memory stop"}]}]}, "model": "meu-modelo"}
        agy = {"ai-memory": {"Stop": [{"command": "ai-memory stop"}]}}
        for dados, configurar in ((claude, configurar_claude), (agy, configurar_antigravity)):
            configurar(dados, Path("/voz/voz.sh"))
            primeira = copy.deepcopy(dados)
            configurar(dados, Path("/voz/voz.sh"))
            self.assertEqual(dados, primeira)
        self.assertEqual(claude["hooks"]["Stop"][0]["hooks"][0]["command"], "ai-memory stop")
        self.assertEqual(claude["model"], "meu-modelo")
        self.assertEqual(agy["ai-memory"]["Stop"][0]["command"], "ai-memory stop")

    def test_instrucao_cria_atualiza_bloco_preservando_regras_existentes(self):
        with tempfile.TemporaryDirectory() as pasta:
            arquivo = Path(pasta) / "AGENTS.md"
            original = "# Minhas regras\n\nNão confunda <!-- voz:inicio --> citado em prosa.\n"
            arquivo.write_text(original, encoding="utf-8")

            configurar_instrucao(arquivo)
            apos_primeira = arquivo.read_text(encoding="utf-8")
            self.assertIn("# Minhas regras", apos_primeira)
            self.assertIn("citado em prosa.", apos_primeira)
            self.assertIn(BLOCO_INSTRUCAO, apos_primeira)
            self.assertEqual(arquivo.with_name("AGENTS.md.voz.bak").read_text(encoding="utf-8"), original)

            # Segunda execução é idempotente e não reescreve
            mtime = arquivo.stat().st_mtime_ns
            configurar_instrucao(arquivo)
            self.assertEqual(arquivo.stat().st_mtime_ns, mtime)

            # Atualização de bloco antigo preserva conteúdo antes e depois
            antigo = "# Antes\n\n<!-- voz:inicio -->\ntexto velho\n<!-- voz:fim -->\n\n# Depois\n"
            atualizado = aplicar_bloco_instrucao(antigo)
            self.assertTrue(atualizado.startswith("# Antes\n\n"))
            self.assertIn(BLOCO_INSTRUCAO, atualizado)
            self.assertTrue(atualizado.endswith("\n# Depois\n"))
            self.assertNotIn("texto velho", atualizado)


if __name__ == "__main__":
    unittest.main()
