#pragma once
#include <QList>
#include <QString>

struct CalculatorStyle {
    QString id, name, resource;
};

const QList<CalculatorStyle>& calculatorStyles();
QString normalizedCalculatorStyle(const QString& id);
QString calculatorStyleResource(const QString& id);
