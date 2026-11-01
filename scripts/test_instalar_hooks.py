import copy
import json
import shlex
import tempfile
import unittest
from pathlib import Path

from instalar_hooks import carregar, configurar_codex, configurar_claude, configurar_antigravity, salvar


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


if __name__ == "__main__":
    unittest.main()
