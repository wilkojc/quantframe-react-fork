import { ActionIcon, Box, Divider, Grid, Group, NumberInput, Stack, Tooltip, Text } from "@mantine/core";
import { useTranslatePage } from "@hooks/index";
import { TextColor } from "@components/textColor";
import { useStockContextContext } from "@contexts/index";
import { PurchaseNewItem } from "./purchase";
import { notifications } from "@mantine/notifications";
import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import { faCheck, faEdit, faHammer, faTrashCan } from "@fortawesome/free-solid-svg-icons";
import { useMutation } from "@tanstack/react-query";
import { CreateStockItemEntryDto, StockItemDto } from "$types/index";
import api from '@api/index';
import { useEffect, useState } from "react";
import { DataTable, DataTableSortStatus } from "mantine-datatable";
import { modals } from "@mantine/modals";
import { paginate, sortArray } from "@utils/index";
import { SearchField } from "@components/searchfield";
interface StockItemsPanelProps {
}
export const StockItemsPanel = ({ }: StockItemsPanelProps) => {
  const useTranslateItemPanel = (key: string, context?: { [key: string]: any }, i18Key?: boolean) => useTranslatePage(`live_trading.tabs.item.${key}`, { ...context }, i18Key)
  const useTranslateNotifaications = (key: string, context?: { [key: string]: any }, i18Key?: boolean) => useTranslateItemPanel(`notifaications.${key}`, { ...context }, i18Key)
  const useTranslateDataGrid = (key: string, context?: { [key: string]: any }, i18Key?: boolean) => useTranslateItemPanel(`datagrid.${key}`, { ...context }, i18Key)
  const useTranslateDataGridColumns = (key: string, context?: { [key: string]: any }, i18Key?: boolean) => useTranslateDataGrid(`columns.${key}`, { ...context }, i18Key);

  const { items } = useStockContextContext();

  // States For DataGrid
  const [page, setPage] = useState(1);
  const pageSizes = [5, 10, 15, 20, 25, 30, 50, 100];
  const [pageSize, setPageSize] = useState(pageSizes[4]);
  const [rows, setRows] = useState<StockItemDto[]>([]);
  const [totalRecords, setTotalRecords] = useState<number>(0);
  const [sortStatus, setSortStatus] = useState<DataTableSortStatus>({ columnAccessor: 'listed_price', direction: 'desc' });
  const [query, setQuery] = useState<string>("");

  // Update DataGrid Rows
  useEffect(() => {
    if (!items)
      return;
    let rivensFilter = items.map((r) => {
      return {
        ...r,
        listed_price: r.listed_price || 0,
      }
    });
    if (query !== "") {
      rivensFilter = rivensFilter.filter((item) => item.name.toLowerCase().includes(query.toLowerCase()));
    }

    setTotalRecords(rivensFilter.length);
    rivensFilter = sortArray([{
      field: sortStatus.columnAccessor,
      direction: sortStatus.direction
    }], rivensFilter);
    rivensFilter = paginate(rivensFilter, page, pageSize);
    setRows(rivensFilter);
  }, [items, query, pageSize, page, sortStatus])

  const [itemPrices, setItemPrices] = useState<Record<string, number>>({});

  // Mutations
  const createStockItemEntryMutation = useMutation((data: CreateStockItemEntryDto) => api.stock.item.create(data), {
    onSuccess: async (data) => {
      notifications.show({
        title: useTranslateNotifaications("createStockItem.title"),
        icon: <FontAwesomeIcon icon={faCheck} />,
        message: useTranslateNotifaications("createStockItem.message", { name: data.name }),
        color: "green"
      });
    },
    onError: () => {

    },
  })

  const sellStockItemEntryMutation = useMutation((data: { id: number, report: boolean, price: number }) => api.stock.item.sell(data.id, data.report, data.price, 1), {
    onSuccess: async (data) => {
      notifications.show({
        title: useTranslateNotifaications("sellStockItem.title"),
        icon: <FontAwesomeIcon icon={faCheck} />,
        message: useTranslateNotifaications("sellStockItem.message", { name: data.name, price: data.listed_price }),
        color: "green"
      });
    },
    onError: () => { },
  })
  const deleteStockItemEntryMutation = useMutation((id: number) => api.stock.item.delete(id), {
    onSuccess: async (data) => {
      notifications.show({
        title: useTranslateNotifaications("deleteStockItem.title"),
        icon: <FontAwesomeIcon icon={faCheck} />,
        message: useTranslateNotifaications("deleteStockItem.message", { name: data.name }),
        color: "green"
      });
    },
    onError: () => {

    },
  })
  const updateItemEntryMutation = useMutation((data: { id: number, riven: Partial<StockItemDto> }) => api.stock.item.update(data.id, data.riven), {
    onSuccess: async (data) => {
      notifications.show({
        title: useTranslateNotifaications("updateStockItem.title"),
        icon: <FontAwesomeIcon icon={faCheck} />,
        message: useTranslateNotifaications("updateStockItem.message", { name: `${data.name}` }),
        color: "green"
      });
    },
    onError: () => { },
  })
  return (
    <Stack >
      <Grid>
        <Grid.Col span={10}>
          <PurchaseNewItem loading={createStockItemEntryMutation.isLoading} onSumit={async (data: CreateStockItemEntryDto) => {
            createStockItemEntryMutation.mutate({
              item_id: data.item_id,
              report: data.report || true,
              price: data.price,
              quantity: data.quantity,
              rank: data.rank
            });
          }} />
        </Grid.Col>
        <Grid.Col span={2}>
          <Stack spacing={1} h={"100%"} w={"100%"}
            sx={{
              display: "flex",
              flexDirection: "column",
              justifyContent: "center",
              alignItems: "center",
            }}
          >
            <TextColor size={"lg"} i18nKey={useTranslateItemPanel("total_purchase_price", undefined, true)} values={{ price: items?.reduce((a, b) => a + (b.price || 0), 0) || 0 }} />
            <TextColor size={"lg"} i18nKey={useTranslateItemPanel("total_listed_price", undefined, true)} values={{ price: items?.reduce((a, b) => a + (b.listed_price || 0), 0) || 0 }} />
          </Stack>
        </Grid.Col>
      </Grid>
      <SearchField value={query} onChange={(text) => setQuery(text)} />
      <DataTable
        striped
        mah={5}
        height={"55vh"}
        withColumnBorders
        records={rows}
        page={page}
        onPageChange={setPage}
        totalRecords={totalRecords}
        recordsPerPage={pageSize}
        recordsPerPageOptions={pageSizes}
        onRecordsPerPageChange={setPageSize}
        sortStatus={sortStatus}
        onSortStatusChange={setSortStatus}

        // define columns
        columns={[
          {
            accessor: 'name',
            title: useTranslateDataGridColumns('name'),
          },
          {
            accessor: 'price',
            title: useTranslateDataGridColumns('price'),
          },
          {
            accessor: 'minium_price',
            title: useTranslateDataGridColumns('minium_price.title'),
            sortable: true,
            render: ({ id, minium_price }) => <Group grow position="apart" >
              <Text>{minium_price || "N/A"}</Text>
              <Box w={25} display="flex" sx={{ justifyContent: "flex-end" }}>
                <Tooltip label={useTranslateDataGridColumns('minium_price.description')}>
                  <ActionIcon size={"sm"} color={"blue.7"} variant="filled" onClick={async (e) => {
                    e.stopPropagation();
                    modals.openContextModal({
                      modal: 'prompt',
                      title: useTranslateDataGridColumns('minium_price.prompt.title'),
                      innerProps: {
                        fields: [
                          {
                            name: 'minium_price',
                            label: useTranslateDataGridColumns('minium_price.prompt.minium_price_label'),
                            value: minium_price || 0,
                            type: 'number',
                          },
                        ],
                        onConfirm: async (data: { minium_price: number }) => {
                          if (!id) return;
                          const { minium_price } = data;
                          updateItemEntryMutation.mutateAsync({ id, riven: { minium_price: minium_price == 0 ? -1 : minium_price } })
                        },
                        onCancel: (id: string) => modals.close(id),
                      },
                    })
                  }} >
                    <FontAwesomeIcon size="xs" icon={faEdit} />
                  </ActionIcon>
                </Tooltip>
              </Box>
            </Group>
          },
          {
            accessor: 'listed_price',
            title: useTranslateDataGridColumns('listed_price'),
            render: ({ listed_price }) => <Text>{listed_price || ""}</Text>
          },
          {
            accessor: 'owned',
            title: useTranslateDataGridColumns('owned'),
          },
          {
            accessor: 'actions',
            width: 275,
            title: useTranslateDataGridColumns('actions.title'),
            render: ({ id, url }) =>
              <Group grow position="center" >
                <NumberInput
                  required
                  size='sm'
                  min={0}
                  max={999}
                  value={itemPrices[url] || ""}
                  onChange={(value) => setItemPrices({ ...itemPrices, [url]: Number(value) })}
                  rightSectionWidth={100}
                  rightSection={
                    <Group spacing={"5px"} mr={0}>
                      <Divider orientation="vertical" />
                      <Tooltip label={useTranslateDataGridColumns('actions.sell')}>
                        <ActionIcon disabled={!itemPrices[url]} loading={sellStockItemEntryMutation.isLoading} color="green.7" variant="filled" onClick={async () => {
                          const price = itemPrices[url];
                          if (!price || price <= 0 || !id) return;
                          await sellStockItemEntryMutation.mutateAsync({ id, price, report: false });
                        }} >
                          <FontAwesomeIcon icon={faHammer} />
                        </ActionIcon>
                      </Tooltip>
                      <Tooltip label={useTranslateDataGridColumns('actions.sell_report')}>
                        <ActionIcon disabled={!itemPrices[url]} loading={sellStockItemEntryMutation.isLoading} color="blue.7" variant="filled" onClick={async () => {
                          const price = itemPrices[url];
                          if (!price || price <= 0 || !id) return;
                          await sellStockItemEntryMutation.mutateAsync({ id, price, report: true });
                        }} >
                          <FontAwesomeIcon icon={faHammer} />
                        </ActionIcon>
                      </Tooltip>
                      <Tooltip label={useTranslateDataGridColumns('actions.delete.title')}>
                        <ActionIcon loading={sellStockItemEntryMutation.isLoading} color="red.7" variant="filled" onClick={async () => {
                          modals.openConfirmModal({
                            title: useTranslateDataGridColumns('actions.delete.title'),
                            children: (<Text>
                              {useTranslateDataGridColumns('actions.delete.message', { name: id })}
                            </Text>),
                            labels: {
                              confirm: useTranslateDataGridColumns('actions.delete.buttons.confirm'),
                              cancel: useTranslateDataGridColumns('actions.delete.buttons.cancel')
                            },
                            confirmProps: { color: 'red' },
                            onConfirm: async () => {
                              if (!id) return;
                              await deleteStockItemEntryMutation.mutateAsync(id);
                            }
                          })
                        }} >
                          <FontAwesomeIcon icon={faTrashCan} />
                        </ActionIcon>
                      </Tooltip>
                    </Group>
                  }
                />
              </Group>
          },
        ]}
      />
    </Stack>)
}