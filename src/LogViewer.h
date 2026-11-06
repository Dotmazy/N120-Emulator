#pragma once
#include <QJsonArray>
#include <QTimer>
#include <QWidget>
class QPlainTextEdit;
class QLineEdit;
class QComboBox;
class QCheckBox;

class LogViewer final : public QWidget {
    Q_OBJECT
  public:
    explicit LogViewer(QWidget* parent = nullptr);
    void append(const QString& text);
    QString text() const;
    void setLimit(int value);

  private:
    void refresh();
    QStringList records;
    QPlainTextEdit* editor;
    QLineEdit* search;
    QComboBox* severity;
    QCheckBox* follow;
    int limit = 1500;
    QTimer refreshTimer;
};
