#include "MainWindow.h"
#include <QtWidgets>
void MainWindow::configure() {
    if (engine.running()) {
        QMessageBox::information(this, "Configuration",
                                 "Stop the engine before changing firmware or LCD identity.");
        return;
    }
    QDialog dialog(this);
    dialog.setWindowTitle("N120 hardware configuration");
    auto layout = new QVBoxLayout(&dialog);
    auto form = new QFormLayout;
    layout->addLayout(form);
    auto fileRow = [&](QString title, QString value, QString filter) {
        auto row = new QWidget;
        auto h = new QHBoxLayout(row);
        h->setContentsMargins(0, 0, 0, 0);
        auto edit = new QLineEdit(value);
        auto browse = new QPushButton("Browse");
        h->addWidget(edit);
        h->addWidget(browse);
        form->addRow(title, row);
        connect(browse, &QPushButton::clicked, &dialog, [&, edit, title, filter] {
            auto path = QFileDialog::getOpenFileName(&dialog, title, edit->text(), filter);
            if (!path.isEmpty())
                edit->setText(path);
        });
        return edit;
    };
    auto internal = fileRow("Internal firmware", config.internal, "Binary firmware (*.bin)");
    auto external = fileRow("External image (optional)", config.external, "Flash image (*.bin)");
    auto persistent = fileRow("Saved flash (load & save)", config.persistent, "Flash image (*.bin)");
    auto renode = fileRow("Renode executable", config.renode, "Renode (renode.exe)");
    auto boot = new QComboBox;
    boot->addItems({"Normal Boot", "BOOT0 / System Memory Bootloader"});
    boot->setCurrentIndex(config.boot0 ? 1 : 0);
    form->addRow("Reset boot selection", boot);
    auto rom = fileRow("System ROM (user supplied)", config.systemRom, "ROM image (*.bin)");
    auto bootNote = new QLabel("BOOT0 is sampled on reset. System boot requires a valid STM32H725 ROM image "
                               "at 0x1FF00000; no STM32 ROM is bundled.");
    bootNote->setWordWrap(true);
    form->addRow(bootNote);
    auto model = new QComboBox;
    for (quint32 id : {0x4E4101u, 0x4E4102u, 0x4E4105u, 0x4E4106u, 0x4E4601u, 0x4E4801u, 0x4E5401u})
        model->addItem(QString("0x%1").arg(id, 6, 16, QChar('0')).toUpper(), id);
    model->setCurrentIndex(qMax(0, model->findData(config.lcdId)));
    form->addRow("LCD hardware identity", model);
    auto slot = new QComboBox;
    slot->addItem("Slot A / full 8 MiB image", 0);
    slot->addItem("Slot B (max 4 MiB)", 0x400000);
    slot->setCurrentIndex(qMax(0, slot->findData(config.externalOffset)));
    form->addRow("External image destination", slot);
    auto info = new QLabel(
        "Saved flash is loaded after the external image. Input uses the physical GPIO matrix. PC keys: "
        "arrows, Enter/EXE, F1/OK, F2/Power, F3/Shift, F4/Alpha, Esc/Back and Home. Type characters normally "
        "for Alpha/Shift sequences. All keys are also clickable on the calculator image.");
    info->setWordWrap(true);
    layout->addWidget(info);
    auto buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    layout->addWidget(buttons);
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    dialog.resize(760, 400);
    if (dialog.exec() == QDialog::Accepted) {
        config.boot0 = boot->currentIndex() == 1;
        config.systemRom = rom->text();
        QSettings().setValue("boot0", config.boot0);
        QSettings().setValue("systemRom", config.systemRom);
        config.internal = internal->text();
        config.external = external->text();
        config.persistent = persistent->text();
        config.renode = renode->text();
        config.lcdId = model->currentData().toUInt();
        config.externalOffset = slot->currentData().toUInt();
        QSettings s;
        s.setValue("renode", config.renode);
        s.setValue("internal", config.internal);
        s.setValue("external", config.external);
        s.setValue("persistent", config.persistent);
        s.setValue("lcd", config.lcdId);
        s.setValue("externalOffset", config.externalOffset);
        startWith(config);
    }
}
