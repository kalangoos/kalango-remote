# Kalango Remote · uma janela GTK 4 que registra o que recebe (clique e texto), para conferir que o controle
# remoto chega ao computador compartilhado. Rode com /usr/bin/python3 (tem o gi).
import sys
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gio, Gtk

# NON_UNIQUE: uma segunda cópia (outro teste) não pode só ativar a primeira e sair
app = Gtk.Application(application_id="os.kalango.RemoteTesteJanela", flags=Gio.ApplicationFlags.NON_UNIQUE)

def ativar(a):
    w = Gtk.ApplicationWindow(application=a, title="teste")
    caixa = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
    campo = Gtk.Entry()
    botao = Gtk.Button(label="clique aqui")
    botao.set_vexpand(True)
    campo.connect("changed", lambda e: print("TEXTO", e.get_text(), flush=True))
    botao.connect("clicked", lambda b: (print("CLIQUE", flush=True), campo.grab_focus()))
    caixa.append(campo)
    caixa.append(botao)
    w.set_child(caixa)
    w.present()

app.connect("activate", ativar)
app.run(None)
