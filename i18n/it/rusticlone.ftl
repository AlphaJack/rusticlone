## Testo per gli utenti. Il codice definisce quattro termini: `{ -name }`, il nome del comando, `{ -app }`, il nome
## dell'app, `{ -env }`, il prefisso delle sue variabili, e `{ -description }`, la descrizione dell'app in Cargo.
## Un messaggio termina senza punto, anche dopo più frasi. Un valore dato dall'utente è scritto com'è; i backtick
## indicano cosa digitare: un comando, un'opzione o una chiave. Un segnaposto è in minuscolo. Le parole contate
## restano in ogni messaggio perché ogni lingua le declina a seconda della frase.

## La riga di comando: il suo aiuto, e il comando collections

help-json = Registra il log in righe JSON
help-verbose = Stampa anche il log di rustic che ogni esecuzione salva
help-all = { label-every-collection }
help-collection = Tutti gli archivi della raccolta con questo nome o profilo di backup
help-dry-run = Mostra cosa farebbe il comando senza eseguirlo né registrarlo
help-repo = L'archivio con questo profilo: il principale di una raccolta o una destinazione di copia
help-due = Esegui ciò che è in scadenza ed esci, per cron: backup pianificati, copie in sospeso o fallite, pulizie e verifiche pianificate
help-backup = Esegui il backup di una raccolta, copialo nelle sue destinazioni di copia e rimuovi gli snapshot vecchi
help-copy = Copia gli snapshot mancanti dall'archivio principale alle destinazioni di copia e rimuovi gli snapshot vecchi
help-prune = Pulisci gli archivi
help-check = Verifica gli archivi: leggi il prossimo sottoinsieme dei loro dati o solo la loro struttura
help-restore = Ripristina le sorgenti da --repo o dall'archivio con gli snapshot più recenti
help-restore-dry-run = Mostra cosa scriverebbe il ripristino e dove. Senza altre opzioni, legge gli snapshot e conta cosa un ripristino sul posto creerebbe, sostituirebbe e manterrebbe
help-restore-at-time = Gli snapshot più recenti presi entro TIME (2026-10-01, "2026-10-01 12:00")
help-restore-snapshot = Questo snapshot dell'archivio --repo (predefinito: il principale)
help-restore-to = Ripristina in DIR. Un --snapshot-path va sotto il suo nome. Ogni sorgente va sotto il suo percorso completo
help-restore-missing = Solo i percorsi mancanti o vuoti. Mai un punto di montaggio il cui dispositivo manca
help-restore-source = Solo la sorgente con questo percorso (predefinito: ogni sorgente)
help-restore-snapshot-path = Solo questo percorso dello snapshot
help-recreate = Ricopia un archivio mancante: il principale dalla sua destinazione più recente, una destinazione dal principale
help-recreate-from = Copia invece da questo profilo. Per il principale: una destinazione di copia. Per una destinazione: il principale o un'altra destinazione
help-notify = Invia subito lo stato di ogni raccolta ai suoi URL di notifica, per provarli
help-collections = Mostra ogni raccolta, o quelle indicate: sorgenti, archivi, pianificazioni, ultime esecuzioni e problemi
help-collections-refresh = Elenca prima gli archivi: i loro snapshot, le dimensioni e quanto sono indietro le copie
help-serve = Avvia ogni esecuzione quando è in scadenza e servi l'interfaccia web
help-serve-open = Apri l'interfaccia web nel browser ed effettua l'accesso
help-install = { $root ->
    [true] Avvia il server ora e all'avvio del computer
   *[false] Avvia il server ora e a ogni accesso
}
help-install-auto-update = Attiva anche gli aggiornamenti automatici in { -name }.env: a ogni avvio, il server aggiorna { -name } e il rustic con cui è testato, dopo averne verificato checksum e attestazioni GitHub
help-uninstall = { $root ->
    [true] Ferma il server e non avviarlo più con il computer
   *[false] Ferma il server e non avviarlo più all'accesso
}
help-uninstall-delete = Elimina anche la cartella di stato: le esecuzioni e il token di accesso. I profili e gli archivi restano
help-update = Sostituisci questo eseguibile con una versione più recente, se esiste
help-update-dry-run = Mostra solo se esiste una versione più recente
help-healthcheck = Esci con 0 se il server risponde (healthcheck di un container)
cli-unselected = seleziona una collezione o un archivio: --all, --collection o --repo
cli-due = in scadenza
cli-notify = notifica
cli-notify-step = inviando lo stato corrente
cli-step-deleted = stato eliminato
cli-step-opened = interfaccia web aperta
cli-login = servizio di login
cli-boot = servizio di avvio
cli-step-placed = eseguibile spostato
cli-step-place = spostando l'eseguibile
cli-step-auto-updated = aggiornamenti automatici attivati
cli-step-checked-rustic = rustic controllato
cli-step-check-rustic = controllando rustic
cli-install = installa
cli-uninstall = disinstalla
cli-update = aggiorna
cli-step-auto-update = attivando gli aggiornamenti automatici
cli-step-install = creando il { $service }
cli-step-uninstall = rimuovendo il { $service }
cli-step-open = aprendo l'interfaccia web
cli-step-delete = eliminando lo stato
cli-step-update = aggiornando rusticlone
cli-step-check-update = cercando una versione più recente
cli-refresh = aggiornamento
cli-ask-keep = Copiare { $files } in { $dir } così che { -name } li trovi da qualsiasi cartella?
collections-directories = Cartelle dei profili
collections-here = { $dir } ({ profile-here })
collections-sources = Sorgenti
collections-repositories = Archivi
collections-issues = Problemi
collections-size = Dimensione
collections-files = File
collections-profile = Profilo
collections-location = Posizione
collections-snapshots = Snapshot
collections-status = Stato
collections-schedules = Pianificazioni
collections-primary = principale
collections-copy-target = destinazione di copia
collections-no-collections = Ancora nessuna raccolta: nessun profilo ha sia un [repository] sia [[backup.snapshots]] con sorgenti. Aggiungili a un profilo in una delle cartelle sopra, o esegui `{ -name } serve --open` per crearne una
collections-other = Altri profili
collections-unknown = { repo-status-unknown }
collections-failing = { $failures ->
    [one] non riuscita
   *[other] non riuscita { $failures } volte di fila
}
# Una pianificazione e quando il suo passaggio è stato eseguito l'ultima volta: `@weekly, ultima volta 2 giorni fa`.
collections-scheduled = { $cron }, ultima volta { $ran }
collections-not-yet = { $cron }, nessuna esecuzione di { $step } finora
collections-unscheduled = manuale
collections-unschedulable = non valida: { $error }
# Lo stato di un archivio con ciò che un elenco ha trovato: `aggiornato, ultimo snapshot 2 ore fa`.
collections-newest = { $status }, ultimo snapshot { $taken }
collections-behind = { $status }, { since-behind }
collections-listed = { $status } (elencato { $age } fa)
collections-kind-backup-profile = Profili di backup
collections-kind-copy-target = Destinazioni di copia
collections-kind-inherited = Già in uso
collections-kind-unused = Inutilizzati
collections-kind-no-sources = Senza sorgenti (il loro [[backup.snapshots]] non ne elenca)
collections-kind-broken = Non validi
collections-warning-hidden = { $read } nasconde { $hidden }: rustic legge solo il primo file con il nome di un profilo. Rinomina o elimina { $hidden }

## Esecuzioni e passaggi: cosa fanno e cosa hanno fatto

run-kind-label = { $kind ->
    [backup] backup
    [copy] copia
    [prune] pulizia
    [check] verifica
    [restore] ripristino
    [test-restore] ripristino (simulazione)
   *[re-create] ricreazione
}
run-kind-doing = { $kind ->
    [backup] backup in corso
    [copy] copia in corso
    [prune] pulizia in corso
    [check] verifica in corso
    [restore] ripristino in corso
    [test-restore] ripristino (simulazione) in corso
   *[re-create] ricreazione in corso
}
trigger-who = { $trigger ->
    [cli] un comando
    [schedule] una pianificazione
   *[web] un utente dell'interfaccia web
}
request-scope-copy-to = su { $target }
request-scope-copy-to-all = su ogni destinazione
request-scope-from = da { $repo }
request-scope-collection = della raccolta
step-snapshot = snapshot su { $repo }
step-copy = copia su { $to }
step-forget = rimozione su { $repo }
step-prune = pulizia di { $repo }
step-check = verifica di { $repo } { $reading }
step-recreate = ricreazione di { $repo }
step-recreate-from = ricreazione di { $repo } da { $from }
step-forgetting = rimozione in corso
status-queued = in coda
status-running = in esecuzione
status-succeeded = riuscito
status-succeeded-with-warnings = riuscito con avvisi
status-failed = fallito
status-skipped = saltato
status-cancelled = annullato
status-interrupted = interrotto
reading-what = { $kind ->
    [structure] solo la struttura
    [subset] sottoinsieme di dati { $reading }
    [all] tutti i dati
   *[sample] { $reading } dei dati a caso
}
blocked-no-targets = nessuna destinazione di copia
blocked-primary-missing = l'archivio principale è scomparso: se il suo disco è scollegato, collegalo; se è stato eliminato, ricrealo
doing-copy-from = da { $from }
doing-waiting-for-listing = in attesa che { $repo } elenchi ciò che è stato scritto
doing-waiting-for = in attesa di { $by }
doing-listing = elenco di { $repo }
result-backed-up = { $count ->
    [0] backup eseguito, nulla è cambiato
   *[other] backup eseguito: { $ids }
}
result-backed-up-new = backup eseguito nel nuovo archivio: { $ids }
result-copied = { $snapshots ->
    [one] copiato 1 snapshot
   *[other] copiati { $snapshots } snapshot
}
result-in-sync = { repo-status-in-sync }
result-forgot = { $snapshots ->
    [0] nulla da rimuovere
    [one] rimosso 1 snapshot
   *[other] rimossi { $snapshots } snapshot
}
result-forgot-held = { $held ->
    [one] mantenuto 1 snapshot scaduto finché non viene copiato
   *[other] mantenuti { $held } snapshot scaduti finché non vengono copiati
}
result-forgot-shrunk = { $shrunk ->
    [one] mantenuto 1 snapshot scaduto perché il più recente è quasi vuoto
   *[other] mantenuti { $shrunk } snapshot scaduti perché il più recente è quasi vuoto
}
result-pruned = pulito
result-pruned-freed = pulito: liberati { $removed }, restano { $remaining } ({ $unused } inutilizzati)
result-verified = verifica completata: { $what }
result-would-back-up = { $files ->
    [one] eseguirebbe il backup di 1 file
   *[other] eseguirebbe il backup di { $files } file
}, { $size }
result-would-copy = { $snapshots ->
    [one] riceverebbe 1 snapshot
   *[other] riceverebbe { $snapshots } snapshot
}
result-would-copy-created = { $snapshots ->
    [one] verrebbe creato e riceverebbe 1 snapshot
   *[other] verrebbe creato e riceverebbe { $snapshots } snapshot
}
result-would-forget = { $snapshots ->
    [0] nulla da rimuovere
    [one] rimuoverebbe 1 snapshot
   *[other] rimuoverebbe { $snapshots } snapshot
}
result-would-prune = pulirebbe
result-would-prune-freed = pulirebbe liberando { $removed } e lasciando { $remaining } ({ $unused } inutilizzati)
result-would-check = verificherebbe: { $what }
result-would-recreate = verrebbe ricreato
result-would-create-primary = il suo primo backup lo creerebbe
result-would-run = eseguirebbe: { $run }
result-nothing-due = nulla in scadenza fino a { $run } il { $at }
result-nothing-scheduled = nulla in scadenza o pianificato
result-primary-created = creato con il chunker di { $from }
result-recreated = { $snapshots ->
    [one] ricreato da { $from }, 1 snapshot
   *[other] ricreato da { $from }, { $snapshots } snapshot
}
result-target-recreated = ricreato
result-listable = può essere elencato, quindi non viene ricreato
result-primary-recreated-empty = ricreato vuoto: nessuna destinazione di copia ha i suoi snapshot
result-test-restored = { $repo }/{ $snapshot }: letti { $size }
# `keep`: i file che già corrispondono allo snapshot. `additional`: i file presenti che lo snapshot non ha.
result-test-restored-plan = { $path }: { $create ->
    [one] creerebbe 1 file
   *[other] creerebbe { $create } file
}, ne sostituirebbe { $replace }, ne manterrebbe { $keep } e ne lascerebbe { $additional } che non sono nello snapshot
result-test-restored-no-counts = { $path }: rustic non ha riportato conteggi
result-restored = ripristinato da { $repo } { $snapshot } ({ $time })
result-restored-to = ripristinato da { $repo } { $snapshot } ({ $time }) in { $to }
result-would-restore = { $files ->
    [one] ripristinerebbe 1 file
   *[other] ripristinerebbe { $files } file
}, { $size }, da { $repo } { $snapshot }
result-would-restore-to = { $files ->
    [one] ripristinerebbe 1 file
   *[other] ripristinerebbe { $files } file
}, { $size }, da { $repo } { $snapshot } in { $to }
result-backup-restored = il backup prosegue dagli snapshot di { $from }
result-left-as-it-is = lasciato com'è: non è vuoto
result-running-already = già in esecuzione, saltato
result-joined = si è unito a { $label }, che { $who } ha avviato nel processo { $pid }: i suoi risultati sono sopra
result-held-back = { $paths } manca o è vuoto, ma l'archivio ne ha snapshot presi da questo computer: ripristinali prima con `{ -name } restore`, o esegui comunque il backup con `{ -name } backup`
result-skipped = { $count ->
    [one] l'ultimo backup non ha potuto leggere 1 elemento
   *[other] l'ultimo backup non ha potuto leggere { $count } elementi
}: { $items }
result-skipped-more = l'ultimo backup non ha potuto leggere { $count } elementi, tra cui { $items }
result-skipped-item = { $path } ({ $error })
result-parent-unreadable = il backup ha riletto ogni file: lo snapshot precedente non ha dati per { $path } ({ $error }). Esegui `{ -name } check`
result-shrunk = { $paths } si è ridotto da { $before } a { $now }: controlla che nulla sia stato eliminato per errore. Gli snapshot più vecchi sono mantenuti finché uno nuovo non raggiunge l'1% di { $before }
result-prune-postponed = pulizia rinviata finché le copie verso di esso non sono finite
result-keeps-everything = saltato: l'archivio mantiene ogni snapshot perché non ha una politica [forget]
result-unavailable = non disponibile: { $error }
result-some-sources = solo { $found } sorgenti su { $of } hanno snapshot: esegui prima il backup per ripristinare le altre
result-unmounted = non ripristinato: il suo punto di montaggio non ha un dispositivo. Montalo e ripristina di nuovo, oppure ripristina in una cartella
result-no-place = non ripristinato sul posto: quel posto non è su questo computer. Ripristinalo in una cartella
result-hooked = non ripristinato sul posto: un ripristino non esegue gli hook di backup che possono montarlo. Ripristinalo in una cartella
result-not-recreated = non ricreato da { $from }: { $error }
result-unmet = saltato perché { $needs } non è riuscito
result-no-target-listed = nessuna destinazione di copia può essere elencata
result-missing-skipped = { $run } { $repo } saltato: { issue-missing }
result-unusable = { $broken ->
    [true] la destinazione di copia { $target } non può essere letta: vedi l'errore del suo profilo
   *[false] la destinazione di copia { $target } non ha un profilo: crea { $target }.toml, o rimuovila dalle destinazioni in [copy]
}

## Rustic: il suo output, e cosa non ha fatto

rustic-untested = rustic { $found } è più recente del { $tested } testato: { -name } può leggerne male l'output. Aggiorna { -name }, o installa rustic { $tested }
# `ended`: come è terminato un comando fallito, `codice di uscita 1`. `said`: l'errore che ha riportato, se c'è.
rustic-failure = { $has_ended ->
    [true] rustic { $command } è fallito ({ $ended })
   *[false] rustic { $command } è uscito con 0 senza farlo
}{ $has_said ->
    [true] : { $said }
   *[false] ; vedi il log di rustic
}
ended-exit = codice di uscita { $code }
ended-killed = ucciso da un segnale
change-identical = identico
change-added = aggiunto
change-removed = rimosso
change-changed = modificato
change-metadata = metadati modificati
change-type = tipo cambiato
# Il motivo di rustic quando non riesce a risolvere il percorso di una sorgente, come lo legge un utente.
output-unresolved = non esiste o non è raggiungibile
# Perché un file tenuto aperto da un altro programma è stato escluso: Windows lo dice nella lingua dell'utente, senza cosa fare.
output-no-password = rustic ha chiesto la password dell'archivio, senza un terminale per digitarla. Imposta password, password-file o password-command nel [repository] del profilo
output-in-use = un altro programma lo tiene aperto e bloccato. Chiudi quel programma prima del backup

## La salute di una raccolta

unconfigured-new = ancora nulla di cui fare il backup: configura un backup
unconfigured-gone = nulla di cui fare il backup: mancano i profili delle raccolte eseguite in precedenza. Rimettili nella cartella dei profili, o configura un nuovo backup
level-word = { $level ->
    [success] in salute
    [warning] in difficoltà
   *[failure] in errore
}
# Cosa fare per un archivio che non può essere elencato, dopo il suo nome.
fix-missing-repo = se il suo disco è scollegato, collegalo; se è stato eliminato, ricrealo con `{ -name } recreate --repo { $repo }`
issue-prunes-on-forget = { $repo } pulisce a ogni rimozione: lento e non pianificato. Togli `prune = true` da [forget] e imposta invece { $key }
issue-setting-ignored = { $repo } { $setting }
issue-no-password = { $repo } non ha una password, quindi rustic la chiederebbe senza che nessuno possa digitarla. Imposta password, password-file o password-command nel suo [repository]
issue-same-repository = { $target } è lo stesso archivio di { $primary }, quindi copiarvi non conserva una seconda copia, e la sua politica [forget] elimina gli snapshot del principale. Dai a { $target } un archivio suo
issue-never-backed-up = nessun backup finora
issue-no-disk-access = macOS non lascia che rustic legga { $source }. In Impostazioni di Sistema, concedi Accesso completo al disco a { $exe }, o al terminale che lo esegue
issue-missing = { $repo } manca: { fix-missing-repo }
issue-stale = { $repo } è in ritardo
issue-undelivered = { notification-undelivered }
issue-since = { $issue }: { $since }. Controlla che il server sia in esecuzione
since-behind = indietro di { $age }
since-last-backup = ultimo backup { $age } fa
last-never = non ancora
repo-status-missing = mancante
repo-status-copy-failing = copia in errore
repo-status-not-created = non creato
repo-status-not-checked = copie non confrontate
repo-status-stale = in ritardo
repo-status-not-backed-up = senza backup
repo-status-behind = copia in sospeso
repo-status-in-sync = aggiornato
repo-status-unknown = sconosciuto

## Notifiche

notification-not-recorded = impossibile registrare la notifica: { $error }
notification-undelivered = notifica fallita: { $error }
notification-warned = notifica inviata con un avviso: { $warning }
notification-notified =
    notifica inviata:
    { $text }
notification-would-notify =
    notifica non inviata (simulazione):
    { $text }
notification-invalid-urls = URL di notifica non valido: { $urls }
notification-no-urls = nessun URL di notifica: imposta { $key }
# Il titolo: `nas: Docs ora è in errore`.
notification-title = { $host }: { $collection } ora è { level-word }
notification-test =
    Notifica di prova di { $name }
    { $emoji } Funziona!
# La prima riga di un recupero: `🟢 È stata in errore per 2 giorni`.
notification-lasted = { $emoji } È stata { level-word } per { $age }
# Dopo un recupero: `🔴 altre 2 raccolte in errore`.
notification-others = { $emoji } { $count ->
    [one] 1 altra raccolta
   *[other] altre { $count } raccolte
} { level-word }

## L'interfaccia web: intestazioni, nomi di colonna ed etichette dei moduli

heading-collections = Raccolte
heading-collection = Raccolta
heading-collection-name = Nome della raccolta
heading-repositories = Archivi
heading-repository = Archivio
heading-sources = Sorgenti
heading-source = Sorgente
heading-folders = Cartelle
heading-snapshots = Snapshot
heading-snapshot = Snapshot
heading-snapshot-path = Percorso nello snapshot
heading-issues = Problemi
heading-runs = Esecuzioni
heading-recent-runs = Esecuzioni recenti
heading-run-details = Dettagli dell'esecuzione
heading-run = Esecuzione
heading-steps = Passaggi
heading-step = Passaggio
heading-commands = Comandi
heading-command = Comando
heading-log = Log
heading-not-found = Non trovato
heading-name = Nome
heading-size = Dimensione
heading-modified = Modificato
heading-actions = Azioni
heading-kind = Tipo
heading-path = Percorso
heading-paths = Percorsi
heading-files = File
heading-added = Aggiunti
heading-previous = Precedente
heading-current = Attuale
heading-file-info = Permessi, proprietario, dimensione, modifica
heading-backed-up = Backup
heading-checked = Verifica
heading-pruned = Pulizia
heading-status = Stato
heading-time = Data
heading-host = Host
heading-index = #
heading-result = Risultato
heading-duration = Durata
heading-start = Inizio
heading-wait = Attesa
heading-trigger = Avviato da
heading-waiting-for = In attesa di
heading-using = In uso
heading-from = Da
heading-to = A
heading-at-time = Alla data
heading-dry-run = Simulazione
heading-token = Token
heading-desktop = Desktop
heading-changes-in = Modifiche in { $snapshot }
heading-find-in = Cerca in { $repo }
heading-set-up = Configura un backup
heading-profiles-saved = Profili rustic salvati
heading-restore-on-new-computer = Per ripristinare su un nuovo computer

## L'interfaccia web: pulsanti e collegamenti

button-back-to-collections = Torna alle raccolte
button-back-up = Esegui il backup
button-back-up-all = Backup di tutto
button-check-all = Verifica tutto
button-prune-all = Pulisci tutto
button-refresh-all = Aggiorna tutto
button-refresh = Aggiorna
button-copy = Copia
button-copy-here = Copia qui
button-copy-this = Copia { $text }
button-check = Verifica
button-prune = Pulisci
button-recreate = Ricrea
button-recreate-this = Ricrea { $repo }
button-create = Crea
button-restore = Ripristina
button-download = Scarica
button-download-previous = Scarica com'era
button-download-current = Scarica com'è
button-dismiss = Ignora
button-test-notification = Invia una notifica di prova
button-stop = Ferma
button-cancel = Annulla
button-changes = Modifiche
button-find = Cerca
button-find-a-file = Cerca un file
button-copy-commands = Copia i comandi
button-copy-log = Copia il log
button-log-in = Accedi
button-log-out = Esci
button-save = Salva
button-add-folder = Aggiungi una cartella
button-remove-folder = Rimuovi questa cartella
button-open-folder = { $os ->
    [macos] Mostra nel Finder
    [windows] Mostra in Esplora file
   *[other] Apri la cartella
}
button-copy-folder-path = Copia il percorso della cartella
button-see-how = Vedi come
button-cron-syntax = Sintassi cron

## L'interfaccia web: parole e stati, nelle tabelle e nei percorsi di navigazione

label-every-collection = Ogni raccolta
label-unknown = { repo-status-unknown }
label-error = errore
label-live = live
label-connecting = connessione...
label-reconnecting = riconnessione...
label-waiting = in attesa
label-stopping = in arresto
label-waiting-for-the-running = in attesa di quella in esecuzione
label-left = incompiuta: il suo processo si è fermato
label-elsewhere = in esecuzione in un altro processo, avviata da { $who }
label-already-queued = { $what } già in coda
label-listing = elenco dei suoi snapshot
label-listing-now = elenco in corso
label-since = Da
label-latest = lo snapshot più recente fra tutti gli archivi
label-snapshot-word = snapshot
label-changes-crumb = modifiche
label-find-crumb = cerca
# L'esecuzione che una pianificazione avvia, prima della pianificazione: `backup 0 3 * * *, prossima volta domani`.
label-backup-schedule = backup
label-copy-schedule = copia dopo il backup
label-check-schedule = verifica
label-prune-schedule = pulizia
label-not-scheduled = { $what }: nessuna pianificazione
label-unschedulable = { $what }: pianificazione non valida ({ $error })
label-scheduled = { $what } { $cron }
label-scheduled-next = { $what } { $cron }, prossima volta { $next }

## L'interfaccia web: frasi, suggerimenti e segnaposto

hint-copied = Copiato
hint-cannot-copy = Impossibile copiare qui
hint-log-selected = Log selezionato: premi Ctrl+C
hint-source-code = Codice sorgente
hint-empty = Vuoto
hint-nothing-changed = Nulla è cambiato
hint-no-runs = Nessuna esecuzione finora
hint-wrong-token = Token errato
hint-nothing-to-restore = Nessuno snapshot da ripristinare
hint-size-on-disk = Su disco: compressi, deduplicati
hint-size-of-files = Dei file
hint-size-backed-up = Dei file all'ultimo backup
hint-files-backed-up = All'ultimo backup
hint-added-on-disk = Nuovi dati su disco, compressi
hint-size-outdated = Misurata prima dell'ultima modifica. Aggiorna per ricalcolarla
hint-holder = L'esecuzione che tiene ciò che questa attende
hint-first-snapshot = Il primo snapshot delle sue sorgenti in questo archivio: tutto il suo contenuto è nuovo
hint-found-nowhere = In nessuno snapshot di { $collection } qui
hint-more-holding = { $more ->
    [one] e 1 altro snapshot con gli stessi file
   *[other] e altri { $more } snapshot con gli stessi file
}
hint-no-snapshot-here = Nessuno snapshot di { $collection } qui
hint-create-profiles-in = Rispondi a qualche domanda, o scrivi i tuoi profili rustic in { $dir }
hint-write-profiles-in = Scrivi i tuoi profili rustic in { $dir }: l'interfaccia web non può salvarli lì
hint-keep-a-copy = Conserva una copia di questi profili lontano da questo computer, per esempio in un gestore di password: ti servono per ripristinare dopo un disastro
hint-now-visible = La tua raccolta ora è in { $app }. Metti i profili delle altre raccolte nella stessa cartella
hint-profiles-step = Metti i tuoi profili in { $dir } o in un'altra cartella
hint-install-step = Installa { $app }:
hint-restore-step = Ripristina ogni raccolta:
hint-resume-step = Poi avvia di nuovo il server di backup:
hint-token-in = Il token è in { $file } su { $host }. Oppure esegui lì `{ -name } serve --open` per accedere senza
hint-token-set = Il token è impostato in { $key } su { $host }. Oppure esegui lì `{ -name } serve --open` per accedere senza
hint-only-if-unlisted = Viene eseguito solo se l'archivio non può essere elencato, e non modifica mai un archivio esistente
hint-created-empty = { result-primary-recreated-empty }
hint-newest-target = la destinazione di copia con gli snapshot più recenti
hint-from-primary = l'archivio principale: ricreato vuoto, i suoi snapshot arrivano con la prossima copia
hint-log-in-first = accedi prima
hint-metrics-only = il token delle metriche apre solo { $metrics } e { $status }: accedi con il token di amministrazione
hint-cross-site = rifiutato: l'ha inviato un altro sito. Apri { -name } direttamente e riprova
hint-name-or-path = parte di un nome o di un percorso
hint-where-it-was = dov'era, o una cartella come /tmp/restored
hint-now = adesso, o un momento come 2026-10-01 12:00
hint-notify = URL di notifica, o niente
notice-started = { $collection }: { $what }
notice-refreshing = { $collection }: aggiornamento in corso
notice-refreshing-repo = { $collection }: aggiornamento di { $repo } in corso
notice-stopping = { $collection }: arresto in corso
notice-dismissed = { $collection }: ignorato
notice-notified = { $collection }: notifica inviata
notice-not-notified = { $collection }: { notification-undelivered }
notice-test-notified = notifica di prova inviata
notice-test-not-notified = notifica di prova fallita: { $error }
notice-folder-not-opened = la cartella non si è aperta: il log del server dice perché
notice-run-failed = { $collection }: { $line }
notice-destination-not-absolute = la destinazione deve essere un percorso assoluto
notice-not-submitted = { $collection }: { $error }
notice-setup-not-saved = non salvato: { $error }
notice-not-running-here = { $collection }: non in esecuzione qui
notice-nothing-to-dismiss = { $collection }: nulla da ignorare
notice-cannot-stop = impossibile fermare: { $error }
notice-cannot-dismiss = impossibile ignorare: { $error }
missing-page = pagina non trovata
missing-asset = risorsa non trovata
missing-run-kind = tipo di esecuzione non trovato
missing-collection = raccolta non trovata
missing-run = esecuzione non trovata
missing-repository = archivio non trovato
missing-snapshot = snapshot non trovato
free-missing = cartella o disco non trovato
free-bytes = { $size } liberi

## L'aiuto delle metriche

metric-process-start-time-seconds = quando il server è stato avviato
metric-build-info = sempre 1, con la versione come etichetta
metric-collection-running = 1 mentre la raccolta ha un'esecuzione in corso
metric-issues = i problemi di questo tipo della raccolta
metric-newest-snapshot-timestamp-seconds = lo snapshot più recente nell'archivio
metric-behind-since-timestamp-seconds = lo snapshot più vecchio che una destinazione di copia non ha
metric-snapshots = gli snapshot della raccolta nell'archivio
metric-stored-bytes = i dati impacchettati dell'archivio
metric-snapshot-files = i file nello snapshot più recente di ogni sorgente
metric-snapshot-processed-bytes = i byte letti dallo snapshot più recente di ogni sorgente
metric-snapshot-added-bytes = i dati impacchettati aggiunti dallo snapshot più recente di ogni sorgente
metric-snapshot-duration-seconds = il tempo impiegato dallo snapshot più recente di ogni sorgente
metric-last-success-timestamp-seconds = l'ultimo successo del passaggio
metric-consecutive-failures = i fallimenti del passaggio dal suo ultimo successo

## Il modulo di configurazione

question-what-to-back-up = Di cosa vuoi fare il backup?
question-where-saved = Dove vuoi conservare il backup?
question-where-copied = Dove vuoi una copia del backup?
question-when-back-up = Ogni quanto vuoi fare il backup?
question-how-long-keep = Per quanto tempo vuoi conservare i backup?
question-how-often-check = Ogni quanto vuoi verificare che il backup non sia danneggiato?
question-where-notify = Dove vuoi ricevere le notifiche sui problemi?
first-run-kind-none = Da nessuna parte
first-run-kind-local = Disco esterno
first-run-kind-smb = Condivisione SMB
first-run-kind-nfs = Condivisione NFS
first-run-kind-s3 = Compatibile con S3
first-run-kind-sftp = SFTP
first-run-kind-webdav = WebDAV
first-run-kind-rest = Server REST
first-run-kind-gdrive = Google Drive
first-run-kind-onedrive = OneDrive
first-run-kind-dropbox = Dropbox
first-run-kind-pcloud = pCloud
first-run-kind-rclone = Altro
group-mounted = Locale
group-direct = Servizi OpenDAL
group-rclone = Servizi Rclone
every-manual = Manualmente
every-hourly = Ogni ora
every-daily = Ogni giorno a mezzanotte
every-weekly = Ogni settimana, la domenica
every-monthly = Ogni mese, il giorno 1
every-custom = Personalizzato
keep-year = Un anno
keep-month = Un mese
keep-all = Per sempre
first-run-key-repository = archivio
field-repository-password = Password dell'archivio
field-server-key = Chiave del server
field-choice-accept = Qualsiasi chiave
field-choice-add = Ricorda la prima chiave
field-choice-strict = Solo chiavi conosciute
field-rclone-password = Password della configurazione di rclone
field-rclone-password-command = Oppure un comando che stampa la password della configurazione di rclone
field-rclone-hint = Configura prima il remote con rclone
field-remote = Remote
field-folder = Cartella
field-path = Percorso
field-server = Server
field-user = Utente
field-private-key = File della chiave privata
field-region = Regione
field-bucket = Bucket
field-access-key = Chiave di accesso
field-secret-key = Chiave segreta
field-password = Password
field-email = Email
field-url = URL
first-run-guide = guida completa alla configurazione: { $url }
first-run-no-choice = nessuna scelta { $choice }
first-run-error-name = il nome deve iniziare con una lettera o una cifra; gli altri caratteri possono essere lettere, cifre, `-`, `_` o `.`
first-run-error-relative = { $path } non è un percorso completo: inizialo dalla radice di un disco, o da ~ per la tua cartella home
first-run-error-no-sources = nessuna cartella di cui fare il backup
first-run-error-no-primary = nessuna posizione principale
first-run-error-no-custom = nessuna pianificazione personalizzata
first-run-error-incomplete = alla posizione { $role ->
    [primary] principale
   *[target] di destinazione
} manca { $field }
first-run-error-not-a-choice = { $field } della posizione { $role ->
    [primary] principale
   *[target] di destinazione
} deve essere uno tra: { $choices }
first-run-error-no-dir = nessuna cartella dei profili in cui si possa scrivere
first-run-error-exists = { $path } esiste già

## Il server, la sua installazione e gli aggiornamenti

server-requested = richiesta da { $by }: { $label }
server-cancelled-from-web = annullato dall'interfaccia web
server-serving = interfaccia web su { $url }. Accedi con { $key }
server-serving-opened = interfaccia web su { $url }, aperta nel browser
server-serving-saved-token = interfaccia web su { $url }. Accedi con il token in { $file } o avvia con --open
server-healthy = { $listen } ok
server-not-opened = impossibile aprirla sul desktop: { $error }
server-reloaded = profili ricaricati{ $changed }
server-not-reloaded = impossibile ricaricare i profili: { $error }
server-not-watching = impossibile osservare i file ({ $error }): ricarica il server dopo aver modificato i profili
server-not-watching-dir = impossibile osservare { $dir }: { $error }
server-dir-not-created = impossibile creare { $dir }: { $error }
server-web-ui-failed = interfaccia web: { $error }
server-no-wall-clock = nessun timer legato all'orologio di sistema ({ $error }): le pianificazioni possono ritardare dopo una sospensione
server-runs-unread = { $runs ->
    [one] 1 esecuzione non mostrata: scritta
   *[other] { $runs } esecuzioni non mostrate: scritte
} in un formato che questa versione non sa leggere
fault-cannot-listen = impossibile ascoltare su { $listen }. Ferma il programma che lo usa, o imposta { $key } su un altro indirizzo
fault-restarting = riavvio con il { -name } aggiornato
fault-no-session = impossibile generare una sessione
fault-no-token = impossibile generare il token di accesso
fault-session-not-kept = impossibile salvare la sessione
fault-no-directory = nessuna cartella { $path } nello snapshot
fault-cannot-run-rustic = impossibile eseguire rustic
fault-not-piped = { rustic-error-not-piped }
fault-stopping = il server si sta fermando
fault-dump-failed = rustic dump è fallito
fault-no-longer-configured = non più configurato
health-error-not-answering = nessuna risposta da { $listen }. Avvialo con `{ -name } serve`
health-error-unhealthy = il server ha risposto { $answer }
autostart-installed = creato il servizio di avvio in { $file }
autostart-installed-at-login = creato il servizio di login in { $file }
autostart-uninstalled = rimosso il servizio in { $file }
autostart-uninstalled-running = rimosso il servizio di login in { $file }. Il server in esecuzione si ferma al logout
autostart-auto-update = aggiornamenti automatici attivati in { $file }
autostart-auto-update-managed = aggiornamento automatico non attivato: { -name } è aggiornato dal gestore pacchetti
autostart-app-refreshed = l'app ora esegue { -name } { $version }, installato dal gestore di pacchetti
autostart-not-installed = nessun servizio da rimuovere
autostart-deleted = eliminato { $dir }
autostart-error-cannot-run = impossibile eseguire { $program }
autostart-error-failed = { $command } è fallito: { $error }
autostart-error-no-profile-dir = nessuna cartella dei profili conosciuta su questo computer
autostart-error-unsupported = nessun gestore di servizi conosciuto su questo computer. Avvia `{ -name } serve` dal tuo servizio o da cron
autostart-blocked = macOS blocca il server all'accesso. Apri Impostazioni di Sistema > Generali > Elementi di login ed estensioni e attiva { $app }
autostart-unit-description = Server { -app }
# Cosa dicono dell'app i desktop e il menu Start di Windows. Le traduzioni lo sostituiscono.
app-description = Backup 3-2-1-1-0 pianificati per computer e server, basati su profili rustic, con interfaccia e notifiche
tray-quit = Esci (sospende i backup automatici)
switch-at-boot = Avvia all'accensione
switch-at-login = Avvia all'accesso
switch-auto-update = Aggiorna automaticamente
update-current = { -name } { $current } è aggiornato
update-available = { -name } { $to } è disponibile, questo è { $from }
update-updated = { -name } aggiornato da { $from } a { $to }
update-packaged = { -name } { $to } è disponibile, { $from } è installato: { $how }
update-with-homebrew = aggiorna con `brew upgrade { -name }`
update-with-scoop = chiudilo dalla barra delle applicazioni e aggiorna con `scoop update { -name }`
update-with-chocolatey = aggiorna come amministratore con `choco upgrade { -name }`
update-with-win-get = aggiorna con `winget upgrade { $id }`
update-with-nix = aggiorna l'input del flake o il pacchetto nella tua configurazione Nix
update-with-docker = scarica di nuovo l'immagine con `docker pull { $image }`
update-with-podman = scarica di nuovo l'immagine con `podman pull { $image }`
update-with-system = aggiorna con il tuo gestore pacchetti o con `sudo { -name } update`
update-not-updated = non aggiornato: { $error }
update-error-checksum = { $file } non corrisponde al suo checksum: scaricalo di nuovo
update-error-no-manifest = la release { $tag } non ha { $manifest }
update-error-no-archive = la release { $tag } non ha un pacchetto per { $target } o un target compatibile
release-error-no-release = nessuna release di rustic per questo computer
release-error-unsigned = la firma di { $archive } non corrisponde alla chiave dei manutentori di rustic. Non usarlo. Scarica di nuovo rustic dalla sua pagina delle release
release-ask-download = rustic non è installato. Scaricare rustic { $version } da { $releases } in { $dir }?
release-ask-replace = Sostituire rustic { $found } in { $dir } con { $version } da { $releases }?
release-downloaded = scaricato rustic { $version } in { $path }
release-downloading = scaricando rustic { $version }
release-declined = rustic non scaricato
release-short-replace = sostituire rustic { $found } con { $version }?
release-short-download = scaricare rustic { $version }?
release-found = trovato rustic { $version } in { $path }
place-copied = spostato { -name } { $version } in { $to }
place-in-place = { -name } { $version } è in { $path }
place-error-write = impossibile scrivere { $path }
place-error-replace = impossibile sostituire { $path }
install-on-path = { $dir } è nel PATH dei nuovi terminali: { $changed }
download-error-download = impossibile scaricare { $url }. Controlla la rete, poi riprova
download-error-status = impossibile scaricare { $url }: HTTP { $status }. Riprova più tardi
download-error-no-executable = { $archive } non contiene { $name }
attest-error-missing = { $archive } non ha un'attestazione di build
attest-error-unattested = { $workflow } non ha costruito { $archive }: { $error }

## Impostazioni

settings-error-no-schedule = { $expr } non è una pianificazione: usa @daily, @weekly, o 5 campi (minuto ora giorno mese giorno della settimana) come `0 3 * * 0`
settings-error-profile-only = { $key } si legge solo dai profili: mettilo nel loro [global.env], o in un profilo che condividono con use-profiles
settings-error-env-file = impossibile leggere { $path }
settings-error-no-config-dir = nessuna cartella di configurazione conosciuta su questo computer
setting-admin-token = Token per l'interfaccia web
setting-auto-update = Aggiorna { -name } e il rustic a ogni riavvio
setting-backup-cron = Pianificazione dei backup (espressione cron, @daily...)
setting-check-cron = Pianificazione delle verifiche dell'archivio
setting-check-subsets = Dati letti da ogni verifica. N legge il prossimo di N sottoinsiemi (`--read-data-subset n/N`). `5%`, `2GiB` o `all` legge quella quantità a caso. 0 legge solo la struttura
setting-listen = Indirizzo dell'interfaccia web
setting-metrics-token = Token per Prometheus e altri sistemi di monitoraggio, da inviare come `Authorization: Bearer`
setting-notify-urls = URL di notifica (Mercurio) per quando lo stato di una raccolta cambia
setting-prune-cron = Pianificazione delle pulizie dell'archivio
setting-rustic-bin = Posizione dell'eseguibile rustic
setting-stall-timeout = Tempo di attesa prima di fermare un comando rustic che non risponde
setting-state-dir = Cartella per il database e i lock delle risorse
settings-ignored = { $key } ignorato: { $why }
unused-unknown = impostazione sconosciuta
unused-environment-only = letto solo dall'ambiente
unused-follows-profiles = una destinazione di copia lo prende dai suoi profili di backup ({ $profiles })

## Errori e log

log-error = errore: { $error }
log-state-afresh = { $path }: { $error }. { -name } parte con uno stato vuoto
log-state-unread = impossibile leggere lo stato: { $error }
log-state-not-kept = impossibile salvare lo stato: { $error }
log-listing-not-kept = impossibile salvare l'elenco di { $repo }: { $error }
log-run-not-saved = impossibile salvare l'esecuzione: { $error }
log-not-measured = impossibile misurare { $repo }: { $error }
log-not-scheduled = impossibile calcolare cosa è in scadenza: { $error }
log-no-profiles = nessun profilo con sorgenti di backup trovato in { $dirs }
log-not-copied = non copiato: { $error }
log-note-left-out = [... { $lines } righe omesse ...]
log-note-cut = ... (i primi { $size } del log sono nel file)
engine-error-no-collection = { $name } non è una raccolta. Le raccolte sono: { $known }
engine-error-no-collections = ancora nessuna raccolta. Esegui `{ -name } serve --open` per crearne una. Oppure aggiungi profili rustic con [[backup.snapshots]] a una di queste cartelle: { $dirs }
engine-error-not-a-source = { $path } non è una sorgente delle raccolte indicate
engine-error-no-repository = { $name } non è un archivio di nessuna raccolta
engine-error-not-a-repository = { $name } non è un archivio di { $collection }
engine-error-not-of-profile = { $name } non è né { $collection } né una delle sue destinazioni di copia
engine-error-backup-of-repository = un backup riguarda un'intera raccolta, non un singolo archivio
engine-error-no-subset = nessun sottoinsieme di dati è stato scelto per la verifica di { $repo }
engine-error-no-subsets = { $key }: { $value } non è un numero di sottoinsiemi, una percentuale, una dimensione o `all`
engine-error-from-itself = impossibile ricreare { $repo } da sé stesso
engine-error-not-backed-up = impossibile fare il backup di { $sources }
engine-error-unexpected = rustic ha registrato lo snapshot { $snapshot } con i percorsi { $paths }, ma le sorgenti si risolvono in { $expected }. { -name } non può copiarlo né ripristinarlo. Dai a ogni sorgente un percorso assoluto fuori dalle altre
engine-error-unlisted-after = impossibile elencare l'archivio dopo il backup. Aggiornalo con `{ -name } collections --refresh`
engine-error-filtered = [snapshot-filter] nel profilo nasconde lo snapshot { $snapshot }, quindi { -name } non può copiarlo né ripristinarlo: cambia il filtro in modo che corrisponda, o rimuovilo
engine-error-primary-unlisted = nessun backup: { $repo } non può essere elencato ({ $error }): { fix-missing-repo }
engine-error-not-recreated = impossibile ricreare { $repo } da { $from }: { $error }
engine-error-folder-away = { $folder }, la cartella di { $repo }, manca: { fix-missing-repo }
engine-error-not-copied-into = impossibile copiare gli snapshot da { $from } in { $repo } ({ $error })
engine-error-not-listed = impossibile elencare { $repo }
engine-error-not-copied-from = impossibile copiare da { $repo }
engine-error-unavailable = { $repo } non è disponibile o è stato eliminato ({ $error }): { fix-missing-repo }
engine-error-unreachable = non raggiungibile
engine-error-not-copied = { $failed } snapshot su { $of } non copiati: { $error }
engine-error-no-snapshot-of-them = { $repo } non ha snapshot di queste sorgenti: esegui prima il backup, o ripristina da un altro archivio con --repo
engine-error-no-matching-snapshot = nessuno snapshot corrispondente in { $repo } o nelle sue destinazioni: usa un --at-time successivo, o esegui prima il backup
engine-error-no-time = impossibile interpretare l'orario { $time }: usa 2026-10-01, "2026-10-01 12:00" o RFC 3339
rustic-error-dry-run-creates = una simulazione non crea archivi
rustic-error-cannot-run = impossibile eseguire rustic: esegui { -name } in un terminale e lascia che scarichi rustic, installa rustic ({ $docs }), o imposta { $key } su uno installato
rustic-error-no-version = impossibile leggere la versione di rustic da { $output }
rustic-error-too-old = rustic { $found } è troppo vecchio: installa { $major }.{ $minor } o successivo
rustic-error-no-snapshot = impossibile leggere lo snapshot { $id } di { $profile }
rustic-error-invalid-profile = { $error }
rustic-error-unreadable = impossibile leggere l'output di `rustic { $command }` (rustic { $version }): installa la versione di rustic testata
rustic-error-stalled = rustic { $command } ucciso: nessun output né attività su disco per { $silent }. Per un archivio lento, aumenta { $key }
rustic-error-no-status = rustic è uscito senza uno stato
rustic-error-not-piped = l'output di rustic non è stato rediretto
rustic-error-percent = una percentuale deve essere maggiore di 0 e al massimo 100
rustic-error-no-size = una dimensione deve essere maggiore di 0
rustic-error-no-answer = nessuna risposta per rustic { $command }
rustic-error-not-a-snapshot-id = { $id } non è un id di snapshot
request-error-not-a-step = { $step } non è un passaggio
state-error-cannot-open = impossibile aprire { $path }
state-error-dequeued = annullato durante l'attesa
state-error-stopped = { $stop } durante l'attesa
state-error-newer = un { -name } più recente ha scritto { $path }: aggiorna { -name }, o sposta { $path } altrove per partire con uno stato vuoto
profile-here = cartella corrente
profile-dirs-or-here = { $dirs } o nella cartella corrente
profile-error-not-found = profilo { $name } non trovato in { $dirs }
profile-error-dry-run = imposta dry-run = true, quindi i suoi backup non salvano nulla: togli dry-run dal profilo
profile-error-slashed-drive = rustic legge { $location } come un backend con il nome della sua unità. Scrivi { $native }
profile-error-backslash-after-scheme = su Windows, rustic legge { $location } come percorso locale a causa della sua barra rovesciata. Scrivi barre normali dopo { $scheme }:

## Tempi

fmt-age-under-a-minute = meno di un minuto
fmt-age-long = molto tempo
fmt-age-ago = { $age } fa
# Un numero di un'unità di tempo: `2 mesi`.
fmt-span = { $unit ->
    [year] { $n ->
        [one] 1 anno
       *[other] { $n } anni
    }
    [month] { $n ->
        [one] 1 mese
       *[other] { $n } mesi
    }
    [day] { $n ->
        [one] 1 giorno
       *[other] { $n } giorni
    }
    [hour] { $n ->
        [one] 1 ora
       *[other] { $n } ore
    }
    [minute] { $n ->
        [one] 1 minuto
       *[other] { $n } minuti
    }
   *[second] { $n ->
        [one] 1 secondo
       *[other] { $n } secondi
    }
}
